fn main() {
    // The same window description the platform shells open through `day_start!`, so every entry
    // point performs the same launch ceremony — the locale catalog included (see `dayapp::window`).
    day::launch(dayapp::window(), dayapp::root);
}
