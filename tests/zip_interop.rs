//! Verify that the hand-written ZIP is compatible with the system tools and
//! that the content round-trips. The fixture deliberately uses Cyrillic names
//! and nesting to exercise UTF-8 filename handling.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use file_transfer::core::archive::archive_dir;

fn unique_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ft_it_{tag}_{}", std::process::id()))
}

#[test]
fn archive_opens_with_system_unzip_and_roundtrips() {
    // `unzip` ships with macOS/Linux by default; skip on other platforms.
    if Command::new("unzip").arg("-v").output().is_err() {
        eprintln!("unzip is unavailable — test skipped");
        return;
    }

    let base = unique_dir("interop");
    let src = base.join("данные");
    std::fs::create_dir_all(src.join("вложенная")).unwrap();
    std::fs::write(src.join("привет.txt"), "file content one").unwrap();
    std::fs::write(src.join("вложенная/file two.bin"), vec![7u8; 5000]).unwrap();

    let zip = base.join("out.zip");
    let cancel = AtomicBool::new(false);
    let n = archive_dir(&src, &zip, 6, &cancel, || {}).unwrap();
    assert_eq!(n, 2, "two files should be packed");

    // 1. Integrity according to the system `unzip`.
    let test = Command::new("unzip").arg("-t").arg(&zip).output().unwrap();
    assert!(
        test.status.success(),
        "unzip -t failed: {}",
        String::from_utf8_lossy(&test.stderr)
    );

    // 2. Extraction and content comparison.
    //    Use a UTF-8-aware tool for extraction: on macOS, `ditto` (built in,
    //    honors the UTF-8 flag); otherwise `unzip`. The legacy Info-ZIP `unzip`
    //    on macOS does not decode UTF-8 names, so it is unsuitable for round-trip.
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
        "extraction failed: {}",
        String::from_utf8_lossy(&extract.stderr)
    );

    let got = std::fs::read_to_string(out.join("данные/привет.txt")).unwrap();
    assert_eq!(got, "file content one");
    let bin = std::fs::read(out.join("данные/вложенная/file two.bin")).unwrap();
    assert_eq!(bin, vec![7u8; 5000]);

    std::fs::remove_dir_all(&base).ok();
}
