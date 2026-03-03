/// Platform-specific utilities implemented per-platform.
///
/// On Windows this wraps the Win32 Common Dialog APIs (via [`rfd`]).
/// On macOS it uses NSOpenPanel / NSSavePanel.
/// On Linux it uses GTK / kdialog / zenity.
pub struct FileDialogs;

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
}
