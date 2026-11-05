pub mod commands;
pub mod db;
pub mod encoding;
pub mod indexer;
pub mod minified;
pub mod parsers;

pub(crate) fn relative_path(root: &std::path::Path, path: &std::path::Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
    #[cfg(windows)]
    {
        relative.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        relative.into_owned()
    }
}
