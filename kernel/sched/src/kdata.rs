//! The KData page (ADR-0010 rule 8, docs/research/kdata.md): the
//! user-readable kernel page Windows Mobile programs read with inline SDK
//! macros, at `0xFFFFC800` within the page at `0xFFFFC000`. The kernel
//! rewrites the per-thread words on every switch.

/// User address of the page that holds KData.
pub const PAGE: u32 = 0xFFFF_C000;
/// `PUserKData`: where the inline macros read.
pub const BASE: u32 = 0xFFFF_C800;

/// Byte offsets from [`BASE`] (measured in the owner's ROM).
pub mod off {
    /// The current thread's TLS slot array (confirmed: 89 modules).
    pub const TLS: usize = 0x000;
    /// The current thread's ID (confirmed: MFC/ATL runtimes).
    pub const CURRENT_THREAD: usize = 0x008;
    /// The current process's ID (CE's macro; not seen in this ROM).
    pub const CURRENT_PROCESS: usize = 0x00C;
}

/// TLS slots per thread (`TlsGetValue` refuses index 64 and up).
pub const TLS_SLOTS: usize = 64;

/// What a thread switch writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadView {
    /// User address of the thread's TLS slot array.
    pub tls: u32,
    /// The thread's ID as programs see it.
    pub thread_id: u32,
    /// Its process's ID.
    pub process_id: u32,
}

/// Writes `t`'s words into the KData page (`page` = the 4 KiB page at
/// [`PAGE`], as the kernel sees it).
pub fn switch_to(page: &mut [u32; 1024], t: ThreadView) {
    let word = |o: usize| (BASE - PAGE) as usize / 4 + o / 4;
    page[word(off::TLS)] = t.tls;
    page[word(off::CURRENT_THREAD)] = t.thread_id;
    page[word(off::CURRENT_PROCESS)] = t.process_id;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_macros_find_their_words() {
        let mut page = [0u32; 1024];
        switch_to(
            &mut page,
            ThreadView {
                tls: 0x0012_3400,
                thread_id: 0x4E,
                process_id: 0x07,
            },
        );
        // The addresses programs read, as words of the page.
        let at = |addr: u32| page[((addr - PAGE) / 4) as usize];
        assert_eq!(at(0xFFFF_C800), 0x0012_3400, "TlsGetValue's pointer");
        assert_eq!(at(0xFFFF_C808), 0x4E, "GetCurrentThreadId");
        assert_eq!(at(0xFFFF_C80C), 0x07, "GetCurrentProcessId");
        assert_eq!(page.iter().filter(|&&w| w != 0).count(), 3, "nothing else");
    }
}
