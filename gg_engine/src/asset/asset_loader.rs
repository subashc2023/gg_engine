use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::renderer::{generate_font_cpu_data, FontCpuData, Texture2D, TextureCpuData, TextureSpecification};
use crate::uuid::Uuid;

// ---------------------------------------------------------------------------
// Channel message types
// ---------------------------------------------------------------------------

pub(crate) enum LoadRequest {
    Texture {
        handle: Uuid,
        path: PathBuf,
        spec: TextureSpecification,
    },
    Font {
        font_key: PathBuf,
        path: PathBuf,
    },
    Shutdown,
}

pub enum LoadResult {
    Texture {
        handle: Uuid,
        data: Result<TextureCpuData, String>,
    },
    Font {
        font_key: PathBuf,
        data: Result<FontCpuData, String>,
    },
}

// ---------------------------------------------------------------------------
// AssetLoader
// ---------------------------------------------------------------------------

const WORKER_COUNT: usize = 2;

/// Internal state created lazily on first request.
struct LoaderInner {
    request_tx: Sender<LoadRequest>,
    result_rx: Receiver<LoadResult>,
    workers: Vec<JoinHandle<()>>,
}

pub struct AssetLoader {
    inner: Option<LoaderInner>,
    pending_textures: HashSet<Uuid>,
    pending_fonts: HashSet<PathBuf>,
}

impl AssetLoader {
    pub fn new() -> Self {
        Self {
            inner: None,
            pending_textures: HashSet::new(),
            pending_fonts: HashSet::new(),
        }
    }

    /// Spawn worker threads on first use.
    fn ensure_started(&mut self) -> &mut LoaderInner {
        self.inner.get_or_insert_with(|| {
            let (request_tx, request_rx) = mpsc::channel::<LoadRequest>();
            let (result_tx, result_rx) = mpsc::channel::<LoadResult>();

            let shared_rx = Arc::new(Mutex::new(request_rx));

            let mut workers = Vec::with_capacity(WORKER_COUNT);
            for i in 0..WORKER_COUNT {
                let rx = Arc::clone(&shared_rx);
                let tx = result_tx.clone();
                let handle = thread::Builder::new()
                    .name(format!("asset-loader-{i}"))
                    .spawn(move || worker_thread_fn(rx, tx))
                    .expect("Failed to spawn asset loader worker thread");
                workers.push(handle);
            }

            log::info!("Asset loader started ({WORKER_COUNT} worker threads)");

            LoaderInner {
                request_tx,
                result_rx,
                workers,
            }
        })
    }

    /// Request async texture loading. Returns false if already pending.
    pub fn request_texture(&mut self, handle: Uuid, path: PathBuf, spec: TextureSpecification) -> bool {
        if !self.pending_textures.insert(handle) {
            return false;
        }
        let inner = self.ensure_started();
        let _ = inner.request_tx.send(LoadRequest::Texture { handle, path, spec });
        true
    }

    /// Request async font loading. Returns false if already pending.
    pub fn request_font(&mut self, font_key: PathBuf) -> bool {
        if !self.pending_fonts.insert(font_key.clone()) {
            return false;
        }
        let path = font_key.clone();
        let inner = self.ensure_started();
        let _ = inner.request_tx.send(LoadRequest::Font { font_key, path });
        true
    }

    /// Non-blocking drain of completed results. Clears pending tracking for
    /// completed items. Returns empty vec if workers were never started.
    pub fn poll_results(&mut self) -> Vec<LoadResult> {
        let inner = match &self.inner {
            Some(inner) => inner,
            None => return Vec::new(),
        };

        let mut results = Vec::new();
        while let Ok(result) = inner.result_rx.try_recv() {
            match &result {
                LoadResult::Texture { handle, .. } => {
                    self.pending_textures.remove(handle);
                }
                LoadResult::Font { font_key, .. } => {
                    self.pending_fonts.remove(font_key);
                }
            }
            results.push(result);
        }
        results
    }

    pub fn is_texture_pending(&self, handle: &Uuid) -> bool {
        self.pending_textures.contains(handle)
    }

    pub fn is_font_pending(&self, font_key: &PathBuf) -> bool {
        self.pending_fonts.contains(font_key)
    }
}

impl Default for AssetLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AssetLoader {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            for _ in &inner.workers {
                let _ = inner.request_tx.send(LoadRequest::Shutdown);
            }
            for worker in inner.workers {
                let _ = worker.join();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Worker thread
// ---------------------------------------------------------------------------

fn worker_thread_fn(rx: Arc<Mutex<Receiver<LoadRequest>>>, tx: Sender<LoadResult>) {
    loop {
        let request = {
            let guard = rx.lock().unwrap();
            guard.recv()
        };

        match request {
            Ok(LoadRequest::Shutdown) | Err(_) => return,
            Ok(LoadRequest::Texture { handle, path, spec }) => {
                let data = Texture2D::load_cpu_data(&path, spec);
                let _ = tx.send(LoadResult::Texture { handle, data });
            }
            Ok(LoadRequest::Font { font_key, path }) => {
                let data = generate_font_cpu_data(&path);
                let _ = tx.send(LoadResult::Font { font_key, data });
            }
        }
    }
}
