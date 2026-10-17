use std::{collections::BTreeMap, io::Read, path::Path, sync::Arc};
pub(super) struct Assets {
    files: BTreeMap<String, (&'static str, Arc<[u8]>)>,
}
impl Assets {
    pub fn load(root: &Path) -> Result<Self, ()> {
        let mut files = BTreeMap::new();
        let mut total = 0;
        Self::visit(root, root, &mut files, &mut total)?;
        if !files.contains_key("/index.html") {
            return Err(());
        }
        Ok(Self { files })
    }
    fn visit(
        root: &Path,
        dir: &Path,
        files: &mut BTreeMap<String, (&'static str, Arc<[u8]>)>,
        total: &mut usize,
    ) -> Result<(), ()> {
        let metadata = std::fs::symlink_metadata(dir).map_err(|_| ())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(());
        }
        for entry in std::fs::read_dir(dir).map_err(|_| ())? {
            let entry = entry.map_err(|_| ())?;
            let metadata = entry.metadata().map_err(|_| ())?;
            if entry.file_type().map_err(|_| ())?.is_symlink() {
                return Err(());
            }
            if metadata.is_dir() {
                Self::visit(root, &entry.path(), files, total)?;
                continue;
            }
            if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 || files.len() >= 2048 {
                return Err(());
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| ())?
                .to_str()
                .ok_or(())?
                .replace('\\', "/");
            if !relative
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b))
                || relative.split('/').any(|c| c == ".." || c.starts_with('.'))
            {
                return Err(());
            }
            let mime = match path.extension().and_then(|s| s.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("svg") => "image/svg+xml",
                Some("png") => "image/png",
                Some("ico") => "image/x-icon",
                Some("woff2") => "font/woff2",
                Some("json") => "application/json",
                _ => return Err(()),
            };
            let mut bytes = Vec::new();
            hiroute_platform::private_fs::open_regular_file(&path)
                .map_err(|_| ())?
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| ())?;
            *total += bytes.len();
            if bytes.len() > 16 * 1024 * 1024 || *total > 64 * 1024 * 1024 {
                return Err(());
            }
            files.insert(format!("/{relative}"), (mime, Arc::from(bytes)));
        }
        Ok(())
    }
    pub fn get(&self, path: &str) -> Option<(&'static str, Arc<[u8]>)> {
        self.files
            .get(if matches!(path, "/" | "/launch") {
                "/index.html"
            } else {
                path
            })
            .cloned()
    }
}
