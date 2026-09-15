use std::fs;
use std::io;
use std::path::Path;

use squish_platform_fs::rename_exclusive;

fn tempdir() -> tempfile::TempDir {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
    fs::create_dir_all(&scratch).expect("workspace scratch root");
    tempfile::Builder::new()
        .prefix("platform-fs-")
        .tempdir_in(scratch)
        .expect("workspace tempdir")
}

#[test]
fn moves_directory_without_changing_contents() {
    let temp = tempdir();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir(&source).expect("source directory");
    fs::write(source.join("payload"), b"complete").expect("payload");

    if !rename_or_skip_unsupported(&source, &destination) {
        return;
    }

    assert!(!source.exists());
    assert_eq!(fs::read(destination.join("payload")).unwrap(), b"complete");
}

#[test]
fn preserves_source_and_existing_empty_directory() {
    assert_existing_destination_is_preserved(|path| fs::create_dir(path), |path| path.is_dir());
}

#[test]
fn preserves_source_and_existing_nonempty_directory() {
    assert_existing_destination_is_preserved(
        |path| {
            fs::create_dir(path)?;
            fs::write(path.join("sentinel"), b"old")
        },
        |path| fs::read(path.join("sentinel")).is_ok_and(|bytes| bytes == b"old"),
    );
}

#[test]
fn preserves_source_and_existing_file() {
    assert_existing_destination_is_preserved(
        |path| fs::write(path, b"old"),
        |path| fs::read(path).is_ok_and(|bytes| bytes == b"old"),
    );
}

fn assert_existing_destination_is_preserved(
    create_destination: impl FnOnce(&Path) -> io::Result<()>,
    destination_unchanged: impl FnOnce(&Path) -> bool,
) {
    let temp = tempdir();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir(&source).expect("source directory");
    fs::write(source.join("payload"), b"new").expect("source payload");
    create_destination(&destination).expect("destination fixture");

    let error = rename_exclusive(&source, &destination).expect_err("must not overwrite");

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(source.join("payload")).unwrap(), b"new");
    assert!(destination_unchanged(&destination));
}

fn rename_or_skip_unsupported(source: &Path, destination: &Path) -> bool {
    match rename_exclusive(source, destination) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::Unsupported => false,
        Err(error) => panic!("exclusive rename: {error}"),
    }
}

#[cfg(unix)]
#[test]
fn preserves_dangling_symlink_destination() {
    use std::os::unix::fs::symlink;

    let temp = tempdir();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir(&source).expect("source directory");
    symlink("missing-target", &destination).expect("destination symlink");

    let error = rename_exclusive(&source, &destination).expect_err("must preserve symlink");

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert!(source.is_dir());
    assert_eq!(
        fs::read_link(destination).unwrap(),
        Path::new("missing-target")
    );
}

#[test]
fn accepts_native_unicode_paths() {
    let temp = tempdir();
    let source = temp.path().join("阶段-🧪");
    let destination = temp.path().join("已提交-🌸");
    fs::create_dir(&source).expect("native source");

    if !rename_or_skip_unsupported(&source, &destination) {
        return;
    }

    assert!(destination.is_dir());
}

#[cfg(unix)]
#[test]
fn accepts_non_utf8_paths() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let temp = tempdir();
    let source = temp.path().join(OsStr::from_bytes(b"source-\xFF"));
    let destination = temp.path().join(OsStr::from_bytes(b"destination-\xFE"));
    fs::create_dir(&source).expect("non-UTF-8 source");

    if !rename_or_skip_unsupported(&source, &destination) {
        return;
    }

    assert!(destination.is_dir());
}

#[test]
fn reports_missing_source() {
    let temp = tempdir();
    let error = rename_exclusive(temp.path().join("missing"), temp.path().join("destination"))
        .expect_err("source is absent");

    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[test]
fn only_one_concurrent_rename_wins() {
    let temp = tempdir();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let destination = temp.path().join("destination");
    fs::create_dir(&first).expect("first source");
    fs::create_dir(&second).expect("second source");
    fs::write(first.join("identity"), b"first").expect("first identity");
    fs::write(second.join("identity"), b"second").expect("second identity");

    let (first_result, second_result) = std::thread::scope(|scope| {
        let first_task = scope.spawn(|| rename_exclusive(&first, &destination));
        let second_task = scope.spawn(|| rename_exclusive(&second, &destination));
        (first_task.join().unwrap(), second_task.join().unwrap())
    });

    let winners = usize::from(first_result.is_ok()) + usize::from(second_result.is_ok());
    if winners == 0
        && first_result
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::Unsupported)
        && second_result
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::Unsupported)
    {
        return;
    }
    assert_eq!(winners, 1);
    let loser = first_result.err().or_else(|| second_result.err()).unwrap();
    assert_eq!(loser.kind(), io::ErrorKind::AlreadyExists);
    let identity = fs::read(destination.join("identity")).expect("winning payload");
    assert!(identity == b"first" || identity == b"second");
}

#[cfg(windows)]
#[test]
fn supports_verbatim_paths_longer_than_max_path() {
    let temp = tempdir();
    let root = temp.path().canonicalize().expect("verbatim temp path");
    let mut parent = root;
    for index in 0..8 {
        parent.push(format!("segment-{index:02}-abcdefghijklmnopqrstuvwxyz"));
        fs::create_dir(&parent).expect("long path segment");
    }
    assert!(parent.as_os_str().len() > 260);
    let source = parent.join("source");
    let destination = parent.join("destination");
    fs::create_dir(&source).expect("long source");

    rename_exclusive(&source, &destination).expect("long rename");

    assert!(destination.is_dir());
}
