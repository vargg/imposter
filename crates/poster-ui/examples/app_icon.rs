//! Writes the packaging icons drawn by `poster_ui::icons::app_icon`:
//! `cargo run -p poster-ui --example app_icon -- crates/poster/icons`.
#![allow(clippy::disallowed_methods)] // a build tool, not the UI thread
#![allow(clippy::unwrap_used)] // developer tool, not app code

use poster_ui::icons::{APP_ICON_FILES, app_icon};

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: app_icon <dir>"));
    std::fs::create_dir_all(&out).unwrap();
    for (file, size) in APP_ICON_FILES {
        let image = image::RgbaImage::from_raw(size, size, app_icon(size)).unwrap();
        let path = out.join(file);
        image.save(&path).unwrap();
        println!("{}", path.display());
    }
}
