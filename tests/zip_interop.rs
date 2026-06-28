//! Проверка совместимости самописного ZIP с системной утилитой `unzip`
//! и round-trip содержимого (включая кириллические имена и вложенность).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use file_transfer::core::archive::archive_dir;

fn unique_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ft_it_{tag}_{}", std::process::id()))
}

#[test]
fn archive_opens_with_system_unzip_and_roundtrips() {
    // unzip есть на macOS/Linux по умолчанию; на других платформах — пропуск.
    if Command::new("unzip").arg("-v").output().is_err() {
        eprintln!("unzip недоступен — тест пропущен");
        return;
    }

    let base = unique_dir("interop");
    let src = base.join("данные");
    std::fs::create_dir_all(src.join("вложенная")).unwrap();
    std::fs::write(src.join("привет.txt"), "содержимое файла раз").unwrap();
    std::fs::write(src.join("вложенная/file two.bin"), vec![7u8; 5000]).unwrap();

    let zip = base.join("out.zip");
    let cancel = AtomicBool::new(false);
    let n = archive_dir(&src, &zip, 6, &cancel, || {}).unwrap();
    assert_eq!(n, 2, "должно быть упаковано 2 файла");

    // 1. Целостность по мнению системного unzip.
    let test = Command::new("unzip").arg("-t").arg(&zip).output().unwrap();
    assert!(
        test.status.success(),
        "unzip -t завершился с ошибкой: {}",
        String::from_utf8_lossy(&test.stderr)
    );

    // 2. Распаковка и сверка содержимого.
    //    Для извлечения берём UTF-8-совместимый инструмент: на macOS — `ditto`
    //    (штатный, уважает UTF-8-флаг), иначе — `unzip`. Старый Info-ZIP `unzip`
    //    в macOS не декодирует UTF-8-имена, поэтому для round-trip он не годится.
    let out = base.join("extracted");
    std::fs::create_dir_all(&out).unwrap();

    let extract = if Command::new("ditto").arg("--version").output().is_ok() {
        Command::new("ditto")
            .args(["-x", "-k"])
            .arg(&zip)
            .arg(&out)
            .output()
            .unwrap()
    } else {
        Command::new("unzip")
            .arg("-o")
            .arg(&zip)
            .arg("-d")
            .arg(&out)
            .output()
            .unwrap()
    };
    assert!(
        extract.status.success(),
        "извлечение не удалось: {}",
        String::from_utf8_lossy(&extract.stderr)
    );

    let got = std::fs::read_to_string(out.join("данные/привет.txt")).unwrap();
    assert_eq!(got, "содержимое файла раз");
    let bin = std::fs::read(out.join("данные/вложенная/file two.bin")).unwrap();
    assert_eq!(bin, vec![7u8; 5000]);

    std::fs::remove_dir_all(&base).ok();
}
