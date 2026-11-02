//! Windows only: embed the OverFilm icon and version info into `OverFilmSubs.exe` and
//! `overfilm-subs.exe` when cross-compiling to `*-windows-gnu` with MinGW's `windres` (how the
//! signed zip is built; `apps/filmcraft/winres.rs`). Everywhere else this does nothing.

#[path = "../filmcraft/winres.rs"]
mod winres;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../filmcraft/winres.rs");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    if !windows || cfg!(windows) {
        return;
    }
    let res =
        winres::Resources { product: "OverFilm", description: "OverFilm 자막 추출", exe: "OverFilmSubs.exe", icon: "../../assets/app-icon/filmcraft.ico" };
    if let Err(e) = winres::embed(&res) {
        println!("cargo:warning=OverFilmSubs built without icon/version resources: {e}");
    }
}
