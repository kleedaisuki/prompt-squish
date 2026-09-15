//! prompt-squish 的小型跨平台文件系统原语。 / Small cross-platform filesystem primitives.
//!
//! 这里集中放置标准库尚未提供、但事务提交需要的操作；不要让平台分支泄漏到领域层。
//! This crate centralizes operations absent from `std` but required by transaction commits;
//! platform branches must not leak into domain code.

#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::io;
use std::path::Path;

/// 在同一文件系统内独占地移动文件或目录，且绝不覆盖目标。
/// Atomically moves a file or directory within one filesystem without ever replacing the target.
///
/// 成功是单次原子重命名。若目标已存在（包括悬空符号链接），返回
/// [`io::ErrorKind::AlreadyExists`]。跨文件系统移动不会退化成复制；调用方会收到底层错误。
/// On success this is one atomic rename. An existing destination (including a dangling symlink)
/// yields [`io::ErrorKind::AlreadyExists`]. Cross-filesystem moves never degrade to a copy and
/// return the underlying operating-system error.
///
/// Linux、Android 与 Apple 平台使用内核的 no-replace rename；Windows 使用 `MoveFileW`。
/// 其他平台明确返回 [`io::ErrorKind::Unsupported`]，而不是模拟一个有竞态的检查后重命名。
/// Linux, Android, and Apple targets use the kernel no-replace rename; Windows uses `MoveFileW`.
/// Other targets return [`io::ErrorKind::Unsupported`] instead of emulating it with a racy check.
/// 远程文件系统可能在提交成功后仍报告传输错误；调用方必须用自己的事务标记核对最终状态。
/// A remote filesystem can report a transport error after committing the move; callers must
/// reconcile an error against their own transaction marker before deciding the final outcome.
///
/// # Examples
///
/// ```no_run
/// # use std::io;
/// # use std::path::Path;
/// use squish_platform_fs::rename_exclusive;
///
/// fn publish(staged: &Path, destination: &Path) -> io::Result<()> {
///     rename_exclusive(staged, destination)
/// }
/// ```
pub fn rename_exclusive(source: impl AsRef<Path>, destination: impl AsRef<Path>) -> io::Result<()> {
    imp::rename_exclusive(source.as_ref(), destination.as_ref())
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
mod imp {
    use std::ffi::OsStr;
    use std::io;
    use std::path::Path;

    use rustix::fs::{CWD, Mode, OFlags, RenameFlags, openat, renameat_with};
    use rustix::io::Errno;

    pub(super) fn rename_exclusive(source: &Path, destination: &Path) -> io::Result<()> {
        let (source_parent, source_name) = split_entry(source)?;
        let (destination_parent, destination_name) = split_entry(destination)?;
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
        let source_parent = openat(CWD, source_parent, flags, Mode::empty()).map_err(map_error)?;
        let destination_parent =
            openat(CWD, destination_parent, flags, Mode::empty()).map_err(map_error)?;

        renameat_with(
            &source_parent,
            source_name,
            &destination_parent,
            destination_name,
            RenameFlags::NOREPLACE,
        )
        .map_err(map_error)
    }

    fn split_entry(path: &Path) -> io::Result<(&Path, &OsStr)> {
        let name = path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "path has no final component")
        })?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        Ok((parent.unwrap_or_else(|| Path::new(".")), name))
    }

    fn map_error(error: Errno) -> io::Error {
        if error == Errno::EXIST || error == Errno::NOTEMPTY {
            return io::Error::new(io::ErrorKind::AlreadyExists, "destination already exists");
        }
        if error == Errno::XDEV {
            return io::Error::new(
                io::ErrorKind::CrossesDevices,
                "source and destination are on different filesystems",
            );
        }
        if error == Errno::NOSYS
            || error == Errno::NOTSUP
            || error == Errno::OPNOTSUPP
            || error == Errno::INVAL
        {
            return io::Error::new(
                io::ErrorKind::Unsupported,
                "exclusive atomic rename is unsupported by this filesystem",
            );
        }
        io::Error::from(error)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_CALL_NOT_IMPLEMENTED, ERROR_FILE_EXISTS,
        ERROR_INVALID_FUNCTION, ERROR_NOT_SAME_DEVICE, ERROR_NOT_SUPPORTED,
    };
    use windows_sys::Win32::Storage::FileSystem::MoveFileW;

    pub(super) fn rename_exclusive(source: &Path, destination: &Path) -> io::Result<()> {
        let source = canonical_entry_path(source)?;
        let destination = canonical_entry_path(destination)?;
        let source = wide_nul(&source)?;
        let destination = wide_nul(&destination)?;

        // SAFETY / 安全性：两个缓冲区均以 NUL 结尾、在调用期间保持存活且不含内部 NUL；
        // MoveFileW only reads them. Both buffers are NUL-terminated, live for the complete call,
        // contain no interior NUL, and MoveFileW only reads them.
        let succeeded = unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) };
        if succeeded != 0 {
            return Ok(());
        }

        let error = io::Error::last_os_error();
        match error.raw_os_error().map(|code| code.cast_unsigned()) {
            Some(ERROR_ALREADY_EXISTS | ERROR_FILE_EXISTS) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "destination already exists",
            )),
            Some(ERROR_ACCESS_DENIED) if destination_exists(&destination) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "destination already exists",
            )),
            Some(ERROR_NOT_SAME_DEVICE) => Err(io::Error::new(
                io::ErrorKind::CrossesDevices,
                "source and destination are on different filesystems",
            )),
            Some(ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED | ERROR_CALL_NOT_IMPLEMENTED) => {
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "exclusive atomic rename is unsupported by this filesystem",
                ))
            }
            _ => Err(error),
        }
    }

    /// 解析父目录而不追踪最后一个路径分量，使符号链接自身可被移动。
    /// Resolves the parent without following the final component, so a symlink itself can move.
    fn canonical_entry_path(path: &Path) -> io::Result<PathBuf> {
        let absolute = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let name = absolute.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "path has no final component")
        })?;
        let parent = absolute.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
        })?;
        Ok(parent.canonicalize()?.join(name))
    }

    /// 创建 Win32 宽字符串并拒绝会造成静默截断的内部 NUL。
    /// Builds a Win32 wide string and rejects interior NULs that would silently truncate it.
    fn wide_nul(path: &Path) -> io::Result<Vec<u16>> {
        let mut wide: Vec<u16> = OsStr::new(path).encode_wide().collect();
        if wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains an interior NUL",
            ));
        }
        wide.push(0);
        Ok(wide)
    }

    fn destination_exists(wide_path: &[u16]) -> bool {
        let path = PathBuf::from(OsString::from_wide(&wide_path[..wide_path.len() - 1]));
        path.symlink_metadata().is_ok()
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    windows
)))]
mod imp {
    use std::io;
    use std::path::Path;

    pub(super) fn rename_exclusive(_source: &Path, _destination: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "exclusive atomic rename is unsupported on this platform",
        ))
    }
}
