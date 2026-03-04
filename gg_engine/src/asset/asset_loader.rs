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

pub struct AssetLoader {
    request_tx: Sender<LoadRequest>,
    result_rx: Receiver<LoadResult>,
    workers: Vec<JoinHandle<()>>,
    pending_textures: HashSet<Uuid>,
    pending_fonts: HashSet<PathBuf>,
}

impl AssetLoader {
    pub fn new() -> Self {
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

        Self {
            request_tx,
            result_rx,
            workers,
            pending_textures: HashSet::new(),
            pending_fonts: HashSet::new(),
        }
    }

    /// Request async texture loading. Returns false if already pending.
    pub fn request_texture(&mut self, handle: Uuid, path: PathBuf, spec: TextureSpecification) -> bool {
        if !self.pending_textures.insert(handle) {
            return false;
        }
        let _ = self.request_tx.send(LoadRequest::Texture { handle, path, spec });
        true
    }

    /// Request async font loading. Returns false if already pending.
    pub fn request_font(&mut self, font_key: PathBuf) -> bool {
        if !self.pending_fonts.insert(font_key.clone()) {
            return false;
        }
        let path = font_key.clone();
        let _ = self.request_tx.send(LoadRequest::Font { font_key, path });
        true
    }

    /// Non-blocking drain of completed results. Clears pending tracking for
    /// completed items.
    pub fn poll_results(&mut self) -> Vec<LoadResult> {
        let mut results = Vec::new();
        while let Ok(result) = self.result_rx.try_recv() {
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

impl Drop for AssetLoader {
    fn drop(&mut self) {
        // Send shutdown sentinel for each worker.
        for _ in &self.workers {
            let _ = self.request_tx.send(LoadRequest::Shutdown);
        }
        // Join all worker threads.
        for worker in self.workers.drain(..) {
            let _ = worker.join();
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
