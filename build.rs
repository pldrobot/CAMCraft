// Embeds the app icon and version info in the Windows exe.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icons/camcraft.ico");
        res.set("ProductName", "CAMCraft");
        res.set("FileDescription", "CAMCraft");
        res.compile().expect("embed windows resources");
    }
}
