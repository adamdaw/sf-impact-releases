//! Files that the tools write or read where a repository could have put
//! something else: a symbolic link, a device or a pipe.
//!
//! A file written is made beside its destination under a name that nothing
//! had and that cannot be guessed, then renamed over the destination: a
//! link there is replaced, never written through. A directory part that
//! has a link in it, below the current directory, is refused. A file read
//! is read only if it is a regular file and not over a size.
//!
//! What this does not stop: a user who can write to the output directory
//! can still change what its names stand for between one call and the
//! next (a directory made a link after it was looked at, the temporary
//! file swapped). `std` has no `openat`, and a dependency for it is not
//! worth it, so the output directory must not be writable by anyone who
//! is not trusted with the output: the same as for most tools that write
//! files. The temporary name is checked again just before the rename, which
//! narrows that window and does not close it.

use crate::diagnostic::shown;
use std::collections::hash_map::RandomState;
use std::ffi::OsString;
use std::fs::{self, File};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

/// The most that a project file or a config file is read of. Real ones
/// are a few kilobytes.
pub const MAX_CONFIG_BYTES: u64 = 16 << 20;

/// How many temporary names are tried before giving up.
const TRIES: u32 = 100;

/// A file being written in place of `path`. Nothing is at `path` until
/// [`Replacement::finish`]; dropped unfinished, the temporary file is
/// removed and `path` is as it was.
pub struct Replacement {
    path: PathBuf,
    temporary: PathBuf,
    file: File,
    finished: bool,
}

impl Replacement {
    /// Starts to replace `path`, which must be a regular file or nothing.
    pub fn new(path: &Path) -> io::Result<Replacement> {
        refuse_links_in_directory(path, &std::env::current_dir()?)?;
        if let Ok(metadata) = fs::symlink_metadata(path)
            && !metadata.is_file()
        {
            return Err(io::Error::other(
                "is not a regular file (a symbolic link, a directory or a device), \
                 so nothing is written there",
            ));
        }
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "names no file"))?;
        let (temporary, file) = create_temporary(directory_of(path), name, &mut unguessable)?;
        Ok(Replacement {
            path: path.to_path_buf(),
            temporary,
            file,
            finished: false,
        })
    }

    /// Puts what was written at the path, on disk first if `sync`.
    pub fn finish(mut self, sync: bool) -> io::Result<()> {
        if sync {
            self.file.sync_all()?;
        }
        // Still the file that was written, and not another that was put at
        // its name: the name is looked at once more before it is renamed.
        let named = fs::symlink_metadata(&self.temporary)?;
        let written = self.file.metadata()?;
        if !named.is_file() || (named.dev(), named.ino()) != (written.dev(), written.ino()) {
            return Err(io::Error::other(
                "its temporary file was changed before it was put in place, so nothing is written",
            ));
        }
        fs::rename(&self.temporary, &self.path)?;
        self.finished = true;
        if sync {
            File::open(directory_of(&self.path))?.sync_all()?;
        }
        Ok(())
    }
}

impl Write for Replacement {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Drop for Replacement {
    fn drop(&mut self) {
        if !self.finished {
            let _ = fs::remove_file(&self.temporary);
        }
    }
}

/// A name part that nothing could have been told: from the hasher that
/// `std` seeds with the operating system's random numbers, and so is
/// different for every call and every process.
fn unguessable() -> String {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}

/// A new file beside the destination, at `.<name>.<suffix>.tmp`. `create_new`
/// is O_CREAT | O_EXCL: it makes a new file and follows no link, whatever
/// is at the name already, and a name that is taken is tried again with
/// another suffix.
fn create_temporary(
    directory: &Path,
    name: &std::ffi::OsStr,
    suffix: &mut dyn FnMut() -> String,
) -> io::Result<(PathBuf, File)> {
    let mut tried = 0;
    loop {
        let mut temporary_name = OsString::from(".");
        temporary_name.push(name);
        temporary_name.push(format!(".{}.tmp", suffix()));
        let temporary = directory.join(temporary_name);
        match File::options()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && tried < TRIES => tried += 1,
            Err(e) => return Err(e),
        }
    }
}

/// Refuses a `path` that has a symbolic link in its directory part, below
/// the current directory `cwd`, which `std` cannot resolve and write
/// through safely: a link there, as `parent -> ../outside`, would have the
/// file made somewhere else than the path says.
///
/// A relative path is walked from its first component, each one looked at
/// with `lstat`; the current directory itself is trusted. An absolute path
/// is walked from the directory that is the current directory (by whatever
/// route it names it: `/home` may be a link to `/var/home`); one that does
/// not go through the current directory is the caller's own, and is not
/// walked. Where `path` goes through a link above the current directory,
/// that is not refused either: it is a setup of the machine, not of the
/// repository (`/tmp` is `/private/tmp` on macOS).
fn refuse_links_in_directory(path: &Path, cwd: &Path) -> io::Result<()> {
    let directory = match path.parent() {
        Some(directory) if !directory.as_os_str().is_empty() => directory,
        _ => return Ok(()),
    };
    let components: Vec<Component> = directory.components().collect();
    let trusted = match directory.is_absolute() {
        false => 0,
        true => {
            let cwd = fs::canonicalize(cwd)?;
            let mut walked = PathBuf::new();
            let mut trusted = components.len();
            for (at, component) in components.iter().enumerate() {
                walked.push(component);
                if fs::canonicalize(&walked).is_ok_and(|walked| walked == cwd) {
                    trusted = at + 1;
                    break;
                }
            }
            trusted
        }
    };
    let mut walked = PathBuf::new();
    for (at, component) in components.iter().enumerate() {
        walked.push(component);
        if at < trusted || !matches!(component, Component::Normal(_)) {
            continue;
        }
        let looked_at = if walked.is_absolute() {
            walked.clone()
        } else {
            cwd.join(&walked)
        };
        if fs::symlink_metadata(&looked_at).is_ok_and(|metadata| metadata.is_symlink()) {
            return Err(io::Error::other(format!(
                "has a symbolic link in its directory ({}), so nothing is written there",
                shown(&walked.to_string_lossy())
            )));
        }
    }
    Ok(())
}

/// The directory that a file at `path` is in.
fn directory_of(path: &Path) -> &Path {
    match path.parent() {
        Some(directory) if !directory.as_os_str().is_empty() => directory,
        _ => Path::new("."),
    }
}

/// Whether `path` is a regular file itself, not a link to one, of at
/// most `cap` bytes.
pub fn regular(path: &Path, cap: u64) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(io::Error::other(
            "is not a regular file (a symbolic link, a directory or a device), so it is not read",
        ));
    }
    if metadata.len() > cap {
        return Err(io::Error::other(format!(
            "is over {cap} bytes, so it is not read"
        )));
    }
    Ok(metadata)
}

/// The bytes of `path`, if it is a regular file itself of at most `cap`
/// bytes: [`regular`], and then the file that is opened must be the file
/// that was looked at.
pub fn read_regular(path: &Path, cap: u64) -> io::Result<Vec<u8>> {
    let looked_at = regular(path, cap)?;
    let file = File::open(path)?;
    let opened = file.metadata()?;
    if (opened.dev(), opened.ino()) != (looked_at.dev(), looked_at.ino()) {
        return Err(io::Error::other(
            "changed while it was opened, so it is not read",
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(opened.len()).unwrap_or(0));
    file.take(cap.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Err(io::Error::other(format!(
            "is over {cap} bytes, so it is not read"
        )));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Dir {
            let dir =
                std::env::temp_dir().join(format!("sf-impact-files-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }

        fn entries(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn replace(path: &Path, content: &[u8]) -> io::Result<()> {
        let mut replacement = Replacement::new(path)?;
        replacement.write_all(content)?;
        replacement.finish(true)
    }

    #[test]
    fn a_file_is_written_where_there_was_none_and_replaced_where_there_was_one() {
        let dir = Dir::new("write");
        let path = dir.0.join("out.jsonl");

        replace(&path, b"first\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first\n");
        replace(&path, b"second\n").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"second\n");
        assert_eq!(dir.entries(), ["out.jsonl"], "no temporary file is left");
    }

    #[test]
    fn a_link_at_the_destination_is_refused_and_its_target_is_left_as_it_was() {
        let dir = Dir::new("link");
        let target = dir.0.join("victim");
        fs::write(&target, b"kept").unwrap();
        let link = dir.0.join("refs.pack");
        symlink(&target, &link).unwrap();
        let dangling = dir.0.join("delta.jsonl");
        symlink(dir.0.join("nothing"), &dangling).unwrap();

        for path in [&link, &dangling] {
            let refused = replace(path, b"written");

            assert!(refused.is_err(), "{path:?}");
        }
        assert_eq!(fs::read(&target).unwrap(), b"kept");
        assert!(!dir.0.join("nothing").exists());
        assert_eq!(dir.entries(), ["delta.jsonl", "refs.pack", "victim"]);
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
    }

    #[test]
    fn a_directory_at_the_destination_is_refused() {
        let dir = Dir::new("directory");
        fs::create_dir(dir.0.join("out")).unwrap();

        assert!(replace(&dir.0.join("out"), b"x").is_err());
        assert_eq!(dir.entries(), ["out"]);
    }

    #[test]
    fn an_unfinished_replacement_leaves_the_destination_as_it_was() {
        let dir = Dir::new("unfinished");
        let path = dir.0.join("out");
        fs::write(&path, b"before").unwrap();

        let mut replacement = Replacement::new(&path).unwrap();
        replacement.write_all(b"half").unwrap();
        drop(replacement);

        assert_eq!(fs::read(&path).unwrap(), b"before");
        assert_eq!(dir.entries(), ["out"]);
    }

    #[test]
    fn a_temporary_name_that_is_taken_is_not_written_through_and_another_is_tried() {
        let dir = Dir::new("taken");
        let target = dir.0.join("victim");
        fs::write(&target, b"kept").unwrap();
        symlink(&target, dir.0.join(".out.first.tmp")).unwrap();
        let mut names = ["first", "second"].into_iter().map(String::from);

        let (temporary, mut file) =
            create_temporary(&dir.0, "out".as_ref(), &mut || names.next().unwrap()).unwrap();
        file.write_all(b"written").unwrap();

        assert_eq!(temporary, dir.0.join(".out.second.tmp"));
        assert_eq!(fs::read(&target).unwrap(), b"kept");
        assert_eq!(fs::read(&temporary).unwrap(), b"written");
    }

    #[test]
    fn a_temporary_name_is_not_the_same_twice() {
        let names: std::collections::HashSet<String> = (0..64).map(|_| unguessable()).collect();

        assert_eq!(names.len(), 64);
        assert!(names.iter().all(|name| name.len() == 16));
    }

    #[test]
    fn a_temporary_file_that_was_swapped_is_not_put_in_place() {
        let dir = Dir::new("swapped");
        let path = dir.0.join("out");
        fs::write(&path, b"before").unwrap();
        let mut replacement = Replacement::new(&path).unwrap();
        replacement.write_all(b"written").unwrap();
        let temporary = replacement.temporary.clone();
        fs::remove_file(&temporary).unwrap();
        fs::write(&temporary, b"swapped in").unwrap();

        assert!(replacement.finish(false).is_err());

        assert_eq!(fs::read(&path).unwrap(), b"before");
        assert_eq!(dir.entries(), ["out"], "the swapped file goes too");
    }

    #[test]
    fn a_link_in_the_directory_part_below_the_current_directory_is_refused() {
        let dir = Dir::new("parent");
        let (inside, outside) = (dir.0.join("inside"), dir.0.join("outside"));
        fs::create_dir_all(inside.join("real/deeper")).unwrap();
        fs::create_dir(&outside).unwrap();
        symlink("../outside", inside.join("parent")).unwrap();
        symlink("real", inside.join("nested")).unwrap();
        symlink("../../../outside", inside.join("real/deeper/up")).unwrap();

        for path in [
            "parent/refs.pack",
            "real/../parent/refs.pack",
            "./parent/refs.pack",
            "nested/deeper/refs.pack",
            "real/deeper/up/refs.pack",
        ] {
            let refused = refuse_links_in_directory(Path::new(path), &inside);
            assert!(refused.is_err(), "{path}");
            let absolute = inside.join(path);
            let refused = refuse_links_in_directory(&absolute, &inside);
            assert!(refused.is_err(), "{absolute:?}");
        }
        assert!(
            refuse_links_in_directory(Path::new("parent/refs.pack"), &inside)
                .unwrap_err()
                .to_string()
                .contains("(parent)")
        );
        for path in [
            "refs.pack",
            "real/refs.pack",
            "real/deeper/refs.pack",
            "./real/refs.pack",
        ] {
            refuse_links_in_directory(Path::new(path), &inside).expect(path);
        }
    }

    #[test]
    fn a_link_above_the_current_directory_or_outside_it_is_the_callers_own() {
        let dir = Dir::new("above");
        let real = dir.0.join("real");
        fs::create_dir_all(real.join("work")).unwrap();
        symlink("real", dir.0.join("home")).unwrap();
        let cwd = dir.0.join("home/work");

        // The current directory reached by a link, and a path that names it
        // by that link or by the real name, or does not go through it.
        for path in [
            dir.0.join("home/work/refs.pack"),
            real.join("work/refs.pack"),
            dir.0.join("home/refs.pack"),
            dir.0.join("home/elsewhere/refs.pack"),
        ] {
            refuse_links_in_directory(&path, &cwd).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        }
        refuse_links_in_directory(Path::new("refs.pack"), &cwd).unwrap();
        // ... and below it a link is a link, by either name.
        symlink("../elsewhere", real.join("work/parent")).unwrap();
        for path in [
            dir.0.join("home/work/parent/refs.pack"),
            real.join("work/parent/refs.pack"),
        ] {
            assert!(refuse_links_in_directory(&path, &cwd).is_err(), "{path:?}");
        }
    }

    #[test]
    fn only_a_regular_file_within_the_cap_is_read() {
        let dir = Dir::new("read");
        let file = dir.0.join("sfdx-project.json");
        fs::write(&file, b"{}").unwrap();
        let link = dir.0.join("link.json");
        symlink(&file, &link).unwrap();
        let zero = dir.0.join("zero.json");
        symlink("/dev/zero", &zero).unwrap();

        assert_eq!(read_regular(&file, 2).unwrap(), b"{}");
        assert!(read_regular(&file, 1).is_err(), "over the cap");
        assert!(read_regular(&link, MAX_CONFIG_BYTES).is_err());
        assert!(read_regular(&zero, MAX_CONFIG_BYTES).is_err());
        assert!(read_regular(Path::new("/dev/zero"), MAX_CONFIG_BYTES).is_err());
        assert!(read_regular(&dir.0, MAX_CONFIG_BYTES).is_err());
        let missing = read_regular(&dir.0.join("none"), MAX_CONFIG_BYTES);
        assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
    }
}
