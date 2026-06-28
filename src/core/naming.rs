//! Генерация имени архива: `<имя_папки>_<YYYY-MM-DD_HHMMSS>.zip`.

use std::path::Path;

/// Имя выходного архива для папки-источника на момент `now`.
///
/// Метка времени до секунды делает коллизии практически невозможными,
/// поэтому перезапись не требуется (см. требование об именовании).
pub fn archive_name(source: &Path, now: chrono::DateTime<chrono::Local>) -> String {
    let stem = source
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "archive".to_string());
    format!("{}_{}.zip", stem, now.format("%Y-%m-%d_%H%M%S"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn builds_name_with_timestamp() {
        let now = chrono::Local.with_ymd_and_hms(2026, 6, 28, 15, 30, 12).unwrap();
        let name = archive_name(Path::new("/tmp/data"), now);
        assert_eq!(name, "data_2026-06-28_153012.zip");
    }

    #[test]
    fn falls_back_for_root() {
        let now = chrono::Local.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let name = archive_name(Path::new("/"), now);
        assert!(name.starts_with("archive_"));
    }
}
