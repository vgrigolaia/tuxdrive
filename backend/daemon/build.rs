fn main() {
    // Proto compilation is deferred to Phase 2 when protoc is available.
    // For now, just signal that changes to the proto file should trigger a rebuild.
    println!("cargo:rerun-if-changed=../../shared/proto/tuxdrive.proto");
    println!("cargo:rerun-if-changed=build.rs");
}
