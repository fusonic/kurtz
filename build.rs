//! Pins the Tabler stylesheet and script. `assets/tabler.lock` holds the version
//! and the SRI hashes. Without `assets/dist/` (development) the admin loads
//! Tabler from the CDN with those hashes as `integrity`. The Docker build
//! downloads the files; they are checked against the hashes here and embedded
//! in the binary.

use std::{env, fs, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha384};

fn main() {
    println!("cargo:rerun-if-changed=assets/tabler.lock");
    println!("cargo:rerun-if-changed=assets/dist/tabler.min.css");
    println!("cargo:rerun-if-changed=assets/dist/tabler.min.js");
    println!("cargo::rustc-check-cfg=cfg(tabler_embedded)");

    let lock = fs::read_to_string("assets/tabler.lock").expect("assets/tabler.lock is missing");
    let value = |key: &str| {
        lock.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_else(|| panic!("{key} is missing in assets/tabler.lock"))
    };
    let (version, sri, js_sri) = (
        value("TABLER_VERSION"),
        value("TABLER_SHA384"),
        value("TABLER_JS_SHA384"),
    );

    let files = [
        ("assets/dist/tabler.min.css", &sri),
        ("assets/dist/tabler.min.js", &js_sri),
    ];
    if files.iter().any(|(path, _)| Path::new(path).exists()) {
        for (path, sri) in files {
            let bytes = fs::read(path).unwrap_or_else(|_| panic!("{path} is missing"));
            let actual = format!("sha384-{}", STANDARD.encode(Sha384::digest(bytes)));
            assert_eq!(&actual, sri, "{path} does not match the hash in assets/tabler.lock");
        }
        println!("cargo:rustc-cfg=tabler_embedded");
    }

    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("tabler.rs");
    fs::write(
        out,
        format!(
            "pub const TABLER_CDN_URL: &str = \"https://cdn.jsdelivr.net/npm/@tabler/core@{version}/dist/css/tabler.min.css\";\npub const TABLER_SRI: &str = \"{sri}\";\npub const TABLER_JS_CDN_URL: &str = \"https://cdn.jsdelivr.net/npm/@tabler/core@{version}/dist/js/tabler.min.js\";\npub const TABLER_JS_SRI: &str = \"{js_sri}\";\n"
        ),
    )
    .unwrap();
}
