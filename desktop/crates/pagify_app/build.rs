fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/pagify-logo.ico");
        res.compile().expect("embedding pagify-logo.ico into the exe");
    }
}
