// Keep the coordinate regression tests host-runnable without opening an
// input device or compiling the Android-only evdev service.
#[path = "../src/input/coordinates.rs"]
mod coordinates;
