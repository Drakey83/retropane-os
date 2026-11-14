//! Notes as its own program (the owner, 2026-11-14): a PE user image the
//! shell loads when Notes opens and releases when it is left
//! (docs/design/app-scope.md, `rp_hal_user::app`). On the PC this is an
//! empty program; the app itself is the library beside it.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

#[cfg(target_os = "none")]
mod program {
    use rp_hal_user::app::AppSlot;

    /// The shell's call: the app's state made in its own `.bss`, handed
    /// back, unless the shell's shapes differ from this program's.
    fn open(o: &mut rp_app::Opening) {
        if !o.fits() {
            return;
        }
        static SLOT: AppSlot<rp_app_notes::Notes> = AppSlot::new();
        let (today, intent) = (o.today, o.intent);
        o.program = SLOT
            .put_with(|| rp_app_notes::opened_for(today, intent))
            .map(|p| p as &mut dyn rp_app::Program);
    }

    rp_hal_user::app_entry!(rp_app::Opening, open);
}

#[cfg(not(target_os = "none"))]
fn main() {}
