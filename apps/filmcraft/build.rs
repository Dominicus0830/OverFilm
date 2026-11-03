//! Windows only: embed the app icon and version info (VERSIONINFO) into the exe, so it shows in
//! Explorer, the taskbar, the Start menu, Alt-Tab and Task Manager (which shows the
//! `FileDescription`, "OverFilm").
//!
//! On a Windows host this uses `winresource`. Cross-compiling for `*-windows-gnu` from Linux or
//! macOS (how OverFilm's installer is built) it uses MinGW's `windres` when it is on the PATH
//! (`x86_64-w64-mingw32-windres`, or `WINDRES`). On every other target this does nothing. A
//! missing resource compiler is a warning, unless `FILMCRAFT_REQUIRE_WINRES=1` (for release
//! builds) turns it into an error.

/// The name shown for the program everywhere Windows reads the exe's resources.
const PRODUCT: &str = "OverFilm";
const DESCRIPTION: &str = "OverFilm";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/filmcraft.ico");
    println!("cargo:rerun-if-env-changed=FILMCRAFT_REQUIRE_WINRES");
    println!("cargo:rerun-if-env-changed=WINDRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    windows::embed_resources();
}

#[cfg(not(windows))]
#[path = "winres.rs"]
mod winres;

fn fail(message: String) {
    if std::env::var_os("FILMCRAFT_REQUIRE_WINRES").is_some() {
        eprintln!("embedding Windows resources failed: {message}");
        std::process::exit(1);
    }
    println!("cargo:warning=OverFilm.exe built without icon/version resources: {message}");
}

#[cfg(windows)]
mod windows {
    pub fn embed_resources() {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/app-icon/filmcraft.ico")
            .set("ProductName", super::PRODUCT)
            .set("FileDescription", super::DESCRIPTION)
            .set("LegalCopyright", "Copyright (c) the FilmCraft and OverFilm contributors. MIT OR Apache-2.0.")
            .set("OriginalFilename", "OverFilm.exe")
            .set("InternalName", "OverFilm");
        if let Err(e) = res.compile() {
            super::fail(e.to_string());
        }
    }
}

/// Cross-compiling for Windows from macOS or Linux: MinGW's `windres` (`winres.rs`, shared with
/// OverFilmSubs).
#[cfg(not(windows))]
mod windows {
    pub fn embed_resources() {
        let res = super::winres::Resources {
            product: super::PRODUCT,
            description: super::DESCRIPTION,
            exe: "OverFilm.exe",
            icon: "../../assets/app-icon/filmcraft.ico",
        };
        if let Err(e) = super::winres::embed(&res) {
            super::fail(e);
        }
    }
}
