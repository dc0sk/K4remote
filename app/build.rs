//! Build script: embed the application icon into the Windows executable
//! (NFR-PKG-01). No-op on other platforms.

fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../packaging/icons/k4remote.ico");
        let _ = res.compile();
    }
}
