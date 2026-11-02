// PROTOTYPE baseline: the smallest exe that links the `windows` crate, so the
// toast and popup sizes can be read as a delta over it.
fn main() {
    let t = unsafe { windows::Win32::System::SystemInformation::GetTickCount() };
    println!("baseline: tick {t}");
}
