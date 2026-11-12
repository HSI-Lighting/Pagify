fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/pagify-logo.ico");
        // What Windows calls the program (Task Manager, "Open with"): it said "pagify_app".
        res.set("FileDescription", "Pagify");
        res.set("ProductName", "Pagify");
        res.set("CompanyName", "HSI Lighting");
        res.compile().expect("embedding pagify-logo.ico into the exe");
    }
}
