use std::path::PathBuf;

const MAX: usize = 10;

/// Recently opened or saved project files, newest first. Stored in the user config folder.
#[derive(Default)]
pub(crate) struct Recent(pub(crate) Vec<PathBuf>);

impl Recent {
    fn file() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("CAMCraft").join("recent.json"))
    }

    pub(crate) fn load() -> Self {
        let list: Vec<PathBuf> = Self::file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self(list.into_iter().filter(|p| p.exists()).take(MAX).collect())
    }

    pub(crate) fn add(&mut self, path: PathBuf) {
        self.0.retain(|p| p != &path);
        self.0.insert(0, path);
        self.0.truncate(MAX);
        // A failed write only loses the history, so it is ignored.
        if let Some(f) = Self::file() {
            if let Some(dir) = f.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string_pretty(&self.0) {
                let _ = std::fs::write(f, s);
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
        if let Some(f) = Self::file() {
            let _ = std::fs::remove_file(f);
        }
    }
}
