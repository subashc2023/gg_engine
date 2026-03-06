use std::io::Write;
use std::path::Path;

/// Write data to a file atomically by writing to a temp file first, then renaming.
///
/// This prevents data corruption if the process crashes mid-write. The rename
/// is atomic on most filesystems (including NTFS on Windows).
pub fn atomic_write(path: impl AsRef<Path>, data: &str) -> std::io::Result<()> {
    let path = path.as_ref();
    let temp_path = path.with_extension("tmp");

    // Write to temp file.
    let mut file = std::fs::File::create(&temp_path)?;
    file.write_all(data.as_bytes())?;
    file.sync_all()?;
    drop(file);

    // Atomic rename (overwrites existing file on Windows via rename).
    // On Windows, std::fs::rename can fail if the target exists, so remove first.
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(&temp_path, path)
}

/// Platform-specific utilities implemented per-platform.
///
/// On Windows this wraps the Win32 Common Dialog APIs (via [`rfd`]).
/// On macOS it uses NSOpenPanel / NSSavePanel.
/// On Linux it uses GTK / kdialog / zenity.
pub struct FileDialogs;

/// Show an error dialog with a single OK button, then return.
pub fn error_dialog(title: &str, message: &str) {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(message)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Show a Yes/No confirmation dialog. Returns `true` if the user clicks Yes.
pub fn confirm_dialog(title: &str, message: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(message)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

impl FileDialogs {
    /// Show a native "Open File" dialog.
    ///
    /// `description` is the human-readable filter label (e.g. `"GGScene files"`).
    /// `extensions` lists file extensions without the dot (e.g. `&["ggscene"]`).
    ///
    /// Returns `None` if the user cancels the dialog.
    pub fn open_file(description: &str, extensions: &[&str]) -> Option<String> {
        rfd::FileDialog::new()
            .add_filter(description, extensions)
            .pick_file()
            .map(|p| p.to_string_lossy().to_string())
    }

    /// Show a native "Open File" dialog starting in a specific directory.
    ///
    /// Like [`open_file`](Self::open_file) but sets the initial directory for
    /// the dialog (e.g. `"assets/textures"`).
    pub fn open_file_in(description: &str, extensions: &[&str], directory: &str) -> Option<String> {
        rfd::FileDialog::new()
            .add_filter(description, extensions)
            .set_directory(directory)
            .pick_file()
            .map(|p| p.to_string_lossy().to_string())
    }

    /// Show a native "Save File" dialog.
    ///
    /// `description` is the human-readable filter label (e.g. `"GGScene files"`).
    /// `extensions` lists file extensions without the dot (e.g. `&["ggscene"]`).
    ///
    /// Returns `None` if the user cancels the dialog.
    pub fn save_file(description: &str, extensions: &[&str]) -> Option<String> {
        rfd::FileDialog::new()
            .add_filter(description, extensions)
            .save_file()
            .map(|p| p.to_string_lossy().to_string())
    }

    /// Show a native "Pick Folder" dialog.
    ///
    /// Returns `None` if the user cancels the dialog.
    pub fn pick_folder() -> Option<String> {
        rfd::FileDialog::new()
            .pick_folder()
            .map(|p| p.to_string_lossy().to_string())
    }
}
