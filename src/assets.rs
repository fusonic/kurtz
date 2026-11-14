//! Static assets embedded in the binary, served under content-hashed names so
//! browsers can cache them forever.

use sha2::{Digest, Sha256};

pub struct Asset {
    pub path: String,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

impl Asset {
    fn new(stem: &str, ext: &str, content_type: &'static str, bytes: &'static [u8]) -> Self {
        let hash = crate::validate::to_hex(&Sha256::digest(bytes));
        Self {
            path: format!("/static/{stem}.{}.{ext}", &hash[..10]),
            content_type,
            bytes,
        }
    }
}

include!(concat!(env!("OUT_DIR"), "/tabler.rs"));

pub struct Assets {
    /// Tabler, embedded in release images; `None` loads it from the CDN.
    pub css: Option<Asset>,
    pub app_css: Asset,
    pub htmx: Asset,
    pub js: Asset,
}

impl Assets {
    /// `href` and, for the CDN copy, the `integrity` hash of the stylesheet.
    pub fn stylesheet(&self) -> (String, Option<String>) {
        match &self.css {
            Some(css) => (css.path.clone(), None),
            None => (TABLER_CDN_URL.to_string(), Some(TABLER_SRI.to_string())),
        }
    }
}

impl Default for Assets {
    fn default() -> Self {
        Self {
            #[cfg(tabler_embedded)]
            css: Some(Asset::new(
                "tabler",
                "css",
                "text/css; charset=utf-8",
                include_bytes!("../assets/dist/tabler.min.css"),
            )),
            #[cfg(not(tabler_embedded))]
            css: None,
            app_css: Asset::new(
                "app",
                "css",
                "text/css; charset=utf-8",
                include_bytes!("../assets/app.css"),
            ),
            htmx: Asset::new(
                "htmx",
                "js",
                "text/javascript; charset=utf-8",
                include_bytes!("../assets/vendor/htmx.min.js"),
            ),
            js: Asset::new(
                "app",
                "js",
                "text/javascript; charset=utf-8",
                include_bytes!("../assets/app.js"),
            ),
        }
    }
}

impl Assets {
    pub fn get(&self, path: &str) -> Option<&Asset> {
        self.css
            .iter()
            .chain([&self.app_css, &self.htmx, &self.js])
            .find(|a| a.path == path)
    }
}
