fn main() {
    #[cfg(feature = "gui")]
    qt_ritual_build::add_resources(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/resources.qrc"));
}