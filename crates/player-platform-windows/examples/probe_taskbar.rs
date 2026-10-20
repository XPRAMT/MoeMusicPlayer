//! Prints what the taskbar prober sees (a diagnostic for the taskbar lyrics): run it before and after an event.
fn main() {
    #[cfg(windows)]
    {
        use player_platform_windows::taskbar::{pick_free_area, AreaChoice, TaskbarProber};
        let prober = TaskbarProber::new().expect("prober");
        let probe = prober.probe().expect("probe");
        println!(
            "bounds={:?} dpi={} auto_hide={} covered={} vertical={}",
            probe.bounds, probe.dpi, probe.auto_hide, probe.covered, probe.vertical
        );
        let mut occupied = probe.occupied.clone();
        occupied.sort_unstable();
        println!("occupied ({}): {:?}", occupied.len(), occupied);
        let scale = |css: i32| css * probe.dpi.max(96) as i32 / 96;
        println!(
            "area={:?}",
            pick_free_area(
                probe.bounds,
                &probe.occupied,
                AreaChoice::Widest,
                scale(8),
                scale(120)
            )
        );
    }
}
