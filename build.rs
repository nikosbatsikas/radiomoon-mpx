fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("packaging/icon.ico");
        res.set("ProductName", "RadioMoon MPX");
        res.set("FileDescription", "RadioMoon MPX transmitter and receiver");
        let _ = res.compile();
    }
}
