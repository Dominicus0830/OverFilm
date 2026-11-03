//! Windows icon + VERSIONINFO for an exe cross-compiled to `*-windows-gnu` from Linux or macOS:
//! writes a `.rc`, compiles it with MinGW's `windres` (`<arch>-w64-mingw32-windres`, or `WINDRES`)
//! and links the object into every binary of the package. Used by the build scripts of the
//! OverFilm editor (`apps/filmcraft/build.rs`) and OverFilmSubs (`apps/overfilm-subs/build.rs`).

use std::path::PathBuf;

/// What Explorer's Details tab and Task Manager show for the exe.
pub struct Resources<'a> {
    pub product: &'a str,
    pub description: &'a str,
    /// `OriginalFilename`, e.g. `OverFilm.exe`.
    pub exe: &'a str,
    /// The `.ico`, relative to the package's manifest directory.
    pub icon: &'a str,
}

/// Escape a string for an `.rc` string literal.
fn rc_str(s: &str) -> String {
    s.replace('"', "\"\"")
}

pub fn embed(res: &Resources) -> Result<(), String> {
    println!("cargo:rerun-if-env-changed=WINDRES");
    println!("cargo:rerun-if-changed={}", res.icon);
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        return Err("cross-compiled for a non-GNU Windows target".into());
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").map_err(|e| e.to_string())?);
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?);
    let icon = manifest.join(res.icon);
    let icon = icon.canonicalize().map_err(|e| format!("{}: {e}", icon.display()))?;
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let mut parts = version.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse::<u16>().unwrap_or(0));
    let (major, minor, patch) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    let vn = format!("{major},{minor},{patch},0");
    let icon_path = rc_str(&icon.display().to_string().replace('\\', "/"));
    let (product, description, exe) = (rc_str(res.product), rc_str(res.description), rc_str(res.exe));
    let internal = rc_str(res.exe.trim_end_matches(".exe"));
    let rc = format!(
        "#pragma code_page(65001)\n\
         1 ICON \"{icon_path}\"\n\
         1 VERSIONINFO\n\
         FILEVERSION {vn}\n\
         PRODUCTVERSION {vn}\n\
         FILEOS 0x40004\n\
         FILETYPE 0x1\n\
         BEGIN\n\
         \x20 BLOCK \"StringFileInfo\"\n\
         \x20 BEGIN\n\
         \x20   BLOCK \"040904B0\"\n\
         \x20   BEGIN\n\
         \x20     VALUE \"CompanyName\", \"Dominicus0830\"\n\
         \x20     VALUE \"ProductName\", \"{product}\"\n\
         \x20     VALUE \"FileDescription\", \"{description}\"\n\
         \x20     VALUE \"FileVersion\", \"{version}\"\n\
         \x20     VALUE \"ProductVersion\", \"{version}\"\n\
         \x20     VALUE \"LegalCopyright\", \"Copyright (c) the FilmCraft and OverFilm contributors. MIT OR Apache-2.0.\"\n\
         \x20     VALUE \"OriginalFilename\", \"{exe}\"\n\
         \x20     VALUE \"InternalName\", \"{internal}\"\n\
         \x20   END\n\
         \x20 END\n\
         \x20 BLOCK \"VarFileInfo\"\n\
         \x20 BEGIN\n\
         \x20   VALUE \"Translation\", 0x409, 1200\n\
         \x20 END\n\
         END\n",
    );
    let rc_path = out.join("overfilm.rc");
    let obj = out.join("overfilm-res.o");
    std::fs::write(&rc_path, rc).map_err(|e| e.to_string())?;
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| format!("{arch}-w64-mingw32-windres"));
    let status = std::process::Command::new(&windres)
        .args(["--input-format=rc", "--output-format=coff", "-c", "65001", "-i"])
        .arg(&rc_path)
        .arg("-o")
        .arg(&obj)
        .status()
        .map_err(|e| format!("{windres}: {e}"))?;
    if !status.success() {
        return Err(format!("{windres} exited with {status}"));
    }
    println!("cargo:rustc-link-arg-bins={}", obj.display());
    Ok(())
}
