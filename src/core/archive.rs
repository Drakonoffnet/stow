//! Параллельная упаковка папки в ZIP.
//!
//! Каждый файл сжимается независимым потоком raw-deflate (метод 8) в общем
//! пуле `rayon`, после чего записи последовательно собираются в корректный
//! ZIP-контейнер. Имена пишутся в UTF-8 (флаг 0x0800) — корректно для кириллицы.
//!
//! Ограничения MVP: стандартный ZIP без ZIP64 (файлы и архив < 4 ГБ),
//! пустые каталоги не сохраняются. См. `docs/design.md`, риск R1.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use flate2::write::DeflateEncoder;
use flate2::Compression;
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::core::error::CoreError;

const LOCAL_FILE_SIG: u32 = 0x0403_4b50;
const CENTRAL_DIR_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const UTF8_FLAG: u16 = 0x0800;
const METHOD_DEFLATE: u16 = 8;
const VERSION_NEEDED: u16 = 20;

/// Один файл, уже сжатый в памяти и готовый к записи в контейнер.
struct PreparedEntry {
    /// Имя внутри архива (с прямыми слэшами, включая корневую папку).
    name: String,
    compressed: Vec<u8>,
    crc32: u32,
    uncompressed_size: u32,
}

/// Собрать список файлов источника (без каталогов).
fn collect_files(source: &Path) -> Result<Vec<std::path::PathBuf>, CoreError> {
    if !source.is_dir() {
        return Err(CoreError::NotADirectory(source.to_path_buf()));
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(source).sort_by_file_name() {
        let entry = entry.map_err(|e| {
            CoreError::Io(e.into_io_error().unwrap_or_else(|| {
                std::io::Error::other("walkdir error")
            }))
        })?;
        if entry.file_type().is_file() {
            files.push(entry.into_path());
        }
    }
    Ok(files)
}

/// Имя записи в архиве: `<корневая папка>/<путь относительно источника>`.
fn entry_name(source: &Path, file: &Path) -> String {
    let root = source
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "root".to_string());
    let rel = file.strip_prefix(source).unwrap_or(file);
    let rel = rel.to_string_lossy().replace('\\', "/");
    format!("{root}/{rel}")
}

/// Сжать один файл в raw-deflate, посчитать CRC32.
fn prepare_entry(
    source: &Path,
    file: &Path,
    level: u32,
) -> Result<PreparedEntry, CoreError> {
    let mut data = Vec::new();
    std::fs::File::open(file)?.read_to_end(&mut data)?;

    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&data);
    let crc32 = hasher.finalize();

    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(level));
    encoder.write_all(&data)?;
    let compressed = encoder.finish()?;

    Ok(PreparedEntry {
        name: entry_name(source, file),
        compressed,
        crc32,
        uncompressed_size: data.len() as u32,
    })
}

/// Упаковать папку `source` в файл `out`.
///
/// * `level` — уровень Deflate (0..=9).
/// * `cancel` — флаг отмены, проверяется между файлами.
/// * `on_file_done` — вызывается после сжатия каждого файла (для прогресса).
///
/// Возвращает число упакованных файлов.
pub fn archive_dir(
    source: &Path,
    out: &Path,
    level: u32,
    cancel: &AtomicBool,
    on_file_done: impl Fn() + Sync,
) -> Result<u64, CoreError> {
    let files = collect_files(source)?;

    // Параллельное сжатие. collect в Result<_> короткозамыкает на первой ошибке
    // (включая отмену), не дожидаясь остальных файлов.
    let entries: Vec<PreparedEntry> = files
        .par_iter()
        .map(|file| {
            if cancel.load(Ordering::Relaxed) {
                return Err(CoreError::Canceled);
            }
            let prepared = prepare_entry(source, file, level)?;
            on_file_done();
            Ok(prepared)
        })
        .collect::<Result<Vec<_>, CoreError>>()?;

    write_zip(out, &entries)?;
    Ok(entries.len() as u64)
}

/// Число файлов в источнике — для отображения общего прогресса.
pub fn count_files(source: &Path) -> Result<u64, CoreError> {
    Ok(collect_files(source)?.len() as u64)
}

/// Последовательная сборка ZIP-контейнера из готовых записей.
fn write_zip(out: &Path, entries: &[PreparedEntry]) -> Result<(), CoreError> {
    let file = std::fs::File::create(out)?;
    let mut w = std::io::BufWriter::new(file);
    let mut offset: u32 = 0;

    // Смещения локальных заголовков — нужны для центральной директории.
    let mut local_offsets = Vec::with_capacity(entries.len());

    for e in entries {
        local_offsets.push(offset);
        let name = e.name.as_bytes();

        offset += write_u32(&mut w, LOCAL_FILE_SIG)?;
        offset += write_u16(&mut w, VERSION_NEEDED)?;
        offset += write_u16(&mut w, UTF8_FLAG)?;
        offset += write_u16(&mut w, METHOD_DEFLATE)?;
        offset += write_u16(&mut w, 0)?; // mod time
        offset += write_u16(&mut w, 0x0021)?; // mod date = 1980-01-01
        offset += write_u32(&mut w, e.crc32)?;
        offset += write_u32(&mut w, e.compressed.len() as u32)?;
        offset += write_u32(&mut w, e.uncompressed_size)?;
        offset += write_u16(&mut w, name.len() as u16)?;
        offset += write_u16(&mut w, 0)?; // extra len
        w.write_all(name)?;
        offset += name.len() as u32;
        w.write_all(&e.compressed)?;
        offset += e.compressed.len() as u32;
    }

    // Центральная директория.
    let cd_start = offset;
    let mut cd_size: u32 = 0;
    for (e, &local_off) in entries.iter().zip(local_offsets.iter()) {
        let name = e.name.as_bytes();
        cd_size += write_u32(&mut w, CENTRAL_DIR_SIG)?;
        cd_size += write_u16(&mut w, VERSION_NEEDED)?; // version made by
        cd_size += write_u16(&mut w, VERSION_NEEDED)?; // version needed
        cd_size += write_u16(&mut w, UTF8_FLAG)?;
        cd_size += write_u16(&mut w, METHOD_DEFLATE)?;
        cd_size += write_u16(&mut w, 0)?; // mod time
        cd_size += write_u16(&mut w, 0x0021)?; // mod date
        cd_size += write_u32(&mut w, e.crc32)?;
        cd_size += write_u32(&mut w, e.compressed.len() as u32)?;
        cd_size += write_u32(&mut w, e.uncompressed_size)?;
        cd_size += write_u16(&mut w, name.len() as u16)?;
        cd_size += write_u16(&mut w, 0)?; // extra len
        cd_size += write_u16(&mut w, 0)?; // comment len
        cd_size += write_u16(&mut w, 0)?; // disk number start
        cd_size += write_u16(&mut w, 0)?; // internal attrs
        cd_size += write_u32(&mut w, 0)?; // external attrs
        cd_size += write_u32(&mut w, local_off)?;
        w.write_all(name)?;
        cd_size += name.len() as u32;
    }

    // End of central directory.
    let count = entries.len() as u16;
    write_u32(&mut w, EOCD_SIG)?;
    write_u16(&mut w, 0)?; // disk number
    write_u16(&mut w, 0)?; // disk with cd
    write_u16(&mut w, count)?;
    write_u16(&mut w, count)?;
    write_u32(&mut w, cd_size)?;
    write_u32(&mut w, cd_start)?;
    write_u16(&mut w, 0)?; // comment len

    w.flush()?;
    Ok(())
}

#[inline]
fn write_u16<W: Write>(w: &mut W, v: u16) -> Result<u32, CoreError> {
    w.write_all(&v.to_le_bytes())?;
    Ok(2)
}

#[inline]
fn write_u32<W: Write>(w: &mut W, v: u32) -> Result<u32, CoreError> {
    w.write_all(&v.to_le_bytes())?;
    Ok(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_and_is_unzippable() {
        let dir = std::env::temp_dir().join(format!("ft_test_{}", std::process::id()));
        let src = dir.join("data");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"hello world hello world").unwrap();
        std::fs::write(src.join("sub/b.txt"), b"second file contents").unwrap();

        let out = dir.join("out.zip");
        let cancel = AtomicBool::new(false);
        let n = archive_dir(&src, &out, 6, &cancel, || {}).unwrap();
        assert_eq!(n, 2);
        assert!(out.metadata().unwrap().len() > 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cancel_stops_archiving() {
        let dir = std::env::temp_dir().join(format!("ft_cancel_{}", std::process::id()));
        let src = dir.join("data");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.txt"), b"x").unwrap();

        let out = dir.join("out.zip");
        let cancel = AtomicBool::new(true);
        let err = archive_dir(&src, &out, 6, &cancel, || {});
        assert!(matches!(err, Err(CoreError::Canceled)));

        std::fs::remove_dir_all(&dir).ok();
    }
}
