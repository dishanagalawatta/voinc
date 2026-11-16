//! Voinc's app icon on Linux: `io.voinc.voinc` in the hicolor theme, the name its
//! desktop entries and window app id use. Packages install it under
//! `/usr/share/icons`; a direct or source install and a downloaded executable
//! have no package, so Voinc writes the same PNGs into the user's icon theme at
//! startup (#652). Each PNG carries an `X-Hex-Managed=true` text chunk, so
//! Voinc replaces, and the installer removes, only icons Voinc wrote: a user's own
//! `io.voinc.voinc.png` or a symlink stays.

use std::io::{ErrorKind, Write};
use std::path::Path;

use color_eyre::eyre::{Result, WrapErr};

use crate::linux::portal::APP_ID;

macro_rules! icon {
    ($size:literal) => {
        (
            $size,
            include_bytes!(concat!(
                "../../packaging/linux/icons/hicolor/",
                $size,
                "x",
                $size,
                "/apps/io.voinc.voinc.png"
            ))
            .as_slice(),
        )
    };
}

/// Every size the packages install, in pixels.
const ICONS: [(&str, &[u8]); 8] = [
    icon!("16"),
    icon!("24"),
    icon!("32"),
    icon!("48"),
    icon!("64"),
    icon!("128"),
    icon!("256"),
    icon!("512"),
];

/// Writes Voinc's icon into the user's hicolor theme, unless a package
/// already installs it; rewrites only icons that differ.
pub(crate) fn install() -> Result<()> {
    let Some(home) = dirs::data_dir() else {
        return Ok(());
    };
    let canonical = |dir: &Path| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.into());
    let own = canonical(&home);
    // Some profiles list the user's own data directory in XDG_DATA_DIRS too.
    let packaged = crate::linux::portal::system_data_dirs()
        .iter()
        .filter(|dir| canonical(dir) != own)
        .any(|dir| icon_path(dir, "48").exists());
    if packaged {
        return Ok(());
    }
    install_in(&home)
}

const MANAGED_MARK: &[u8] = b"X-Hex-Managed=true";

/// The mark's own tEXt chunk: `Comment` text, then the mark.
const MANAGED_MARK_CHUNK: &[u8] = b"Comment\0X-Hex-Managed=true";

/// The icon bytes without the mark chunk: the exact bytes builds older than
/// the mark wrote into the user's theme. A file that equals them is Voinc's
/// own icon that only lacks its mark, so it is replaced; a user's own icon
/// never byte-matches the art.
fn unmarked(png: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(png.len());
    let (signature, mut rest) = png.split_at(png.len().min(8));
    out.extend_from_slice(signature);
    while rest.len() >= 12 {
        let length = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
        if rest.len() < 12 + length {
            break; // malformed tail: keep it as it is
        }
        let (chunk, remainder) = rest.split_at(12 + length);
        if !is_mark_chunk(chunk) {
            out.extend_from_slice(chunk);
        }
        rest = remainder;
    }
    out.extend_from_slice(rest);
    out
}

fn is_mark_chunk(chunk: &[u8]) -> bool {
    chunk[4..8] == *b"tEXt"
        && chunk.len() == 12 + MANAGED_MARK_CHUNK.len()
        && chunk[8..8 + MANAGED_MARK_CHUNK.len()] == *MANAGED_MARK_CHUNK
}

fn install_in(data: &Path) -> Result<()> {
    let mut wrote = false;
    for (size, bytes) in ICONS {
        let path = icon_path(data, size);
        let replace = match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => true,
            Err(error) => {
                return Err(error).wrap_err_with(|| format!("could not read {}", path.display()));
            }
            Ok(metadata) if metadata.is_file() => {
                let current = std::fs::read(&path)
                    .wrap_err_with(|| format!("could not read {}", path.display()))?;
                current != bytes && (managed(&current) || current == unmarked(bytes))
            }
            Ok(_) => false,
        };
        if replace {
            write(&path, bytes).wrap_err_with(|| format!("could not write {}", path.display()))?;
            wrote = true;
        }
    }
    if wrote {
        // Icon caches watch the theme directory itself, not its subdirectories.
        let theme = data.join("icons/hicolor");
        std::fs::File::open(&theme)
            .and_then(|theme| theme.set_modified(std::time::SystemTime::now()))
            .wrap_err_with(|| format!("could not touch {}", theme.display()))?;
    }
    Ok(())
}

fn managed(png: &[u8]) -> bool {
    png.windows(MANAGED_MARK.len())
        .any(|window| window == MANAGED_MARK)
}

fn icon_path(data: &Path, size: &str) -> std::path::PathBuf {
    data.join(format!("icons/hicolor/{size}x{size}/apps/{APP_ID}.png"))
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| color_eyre::eyre::eyre!("{} has no directory", path.display()))?;
    std::fs::create_dir_all(directory)?;
    let staged = directory.join(format!(".{APP_ID}.png.{}", std::process::id()));
    match std::fs::remove_file(&staged) {
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    let mut file = std::fs::File::options()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&staged, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_size_is_a_png_of_its_size() {
        for (size, bytes) in ICONS {
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
            let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
            let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
            assert_eq!(
                (width.to_string(), height.to_string()),
                (size.into(), size.into())
            );
        }
    }

    #[test]
    fn every_icon_carries_the_managed_mark() {
        assert!(ICONS.iter().all(|(_, bytes)| managed(bytes)));
        // The strip is exact: the unmarked bytes are the art, nothing else.
        for (_, bytes) in ICONS {
            let plain = unmarked(bytes);
            assert_ne!(plain, bytes);
            assert!(managed(bytes) && !managed(&plain));
            assert_eq!(plain, unmarked(&plain), "stripping twice changes nothing");
        }
    }

    #[test]
    fn a_pre_mark_install_grows_its_mark() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data = std::env::temp_dir().join(format!("voinc-icons-premark-{unique}"));
        // What builds older than the mark wrote: the art without its chunk.
        let path = data.join("icons/hicolor/48x48/apps/io.voinc.voinc.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, unmarked(ICONS[3].1)).unwrap();
        install_in(&data).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), ICONS[3].1);
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn icons_hex_did_not_write_stay() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data = std::env::temp_dir().join(format!("voinc-icons-own-{unique}"));
        let own = data.join("icons/hicolor/48x48/apps/io.voinc.voinc.png");
        let linked = data.join("icons/hicolor/64x64/apps/io.voinc.voinc.png");
        std::fs::create_dir_all(own.parent().unwrap()).unwrap();
        std::fs::create_dir_all(linked.parent().unwrap()).unwrap();
        std::fs::write(&own, b"mine").unwrap();
        std::os::unix::fs::symlink(&own, &linked).unwrap();
        // An older icon Voinc wrote is replaced.
        let older = data.join("icons/hicolor/32x32/apps/io.voinc.voinc.png");
        std::fs::create_dir_all(older.parent().unwrap()).unwrap();
        std::fs::write(&older, b"old X-Hex-Managed=true").unwrap();
        install_in(&data).unwrap();
        assert_eq!(std::fs::read(&own).unwrap(), b"mine");
        assert!(linked.is_symlink());
        assert_eq!(std::fs::read(&older).unwrap(), ICONS[2].1);
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn icons_land_in_the_users_hicolor_theme() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data = std::env::temp_dir().join(format!("voinc-icons-{unique}"));
        install_in(&data).unwrap();
        let written =
            std::fs::read(data.join("icons/hicolor/48x48/apps/io.voinc.voinc.png")).unwrap();
        assert_eq!(written, ICONS[3].1);
        std::fs::remove_dir_all(data).unwrap();
    }
}
