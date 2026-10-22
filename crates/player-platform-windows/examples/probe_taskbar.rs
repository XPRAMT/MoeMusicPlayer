//! Prints what the taskbar prober sees on every taskbar (a diagnostic for the taskbar lyrics, ADR 0018 / 0031): run
//! it before and after an event.
fn main() {
    #[cfg(windows)]
    {
        use player_platform_windows::taskbar::{
            list_monitors, list_taskbars, lyrics_area, TaskbarProber,
        };
        for monitor in list_monitors() {
            println!(
                "monitor {} {} primary={} bounds={:?} name={:?} id={}",
                monitor.number,
                monitor.device,
                monitor.primary,
                monitor.bounds,
                monitor.name,
                monitor.id
            );
        }
        let prober = TaskbarProber::new().expect("prober");
        for taskbar in list_taskbars() {
            println!(
                "taskbar hwnd={} primary={} monitor={}",
                taskbar.hwnd, taskbar.primary, taskbar.monitor_device
            );
            let probe = prober.probe_taskbar(&taskbar).expect("probe");
            println!(
                "  bounds={:?} dpi={} auto_hide={} covered={} vertical={}",
                probe.bounds, probe.dpi, probe.auto_hide, probe.covered, probe.vertical
            );
            let mut occupied = probe.occupied.clone();
            occupied.sort_unstable();
            println!("  occupied ({}): {:?}", occupied.len(), occupied);
            println!("  area={:?}", lyrics_area(&probe, 8, 120));
        }
    }
}
