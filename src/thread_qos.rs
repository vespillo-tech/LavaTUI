//! macOS scheduling hints. Workers explicitly shed inherited UI priority.

#[cfg(target_os = "macos")]
pub struct UiPriority {
    original: libc::qos_class_t,
    relative: libc::c_int,
    active: Option<bool>,
}

#[cfg(target_os = "macos")]
impl UiPriority {
    pub fn new() -> Self {
        use libc::qos_class_t::QOS_CLASS_DEFAULT;
        let mut original = QOS_CLASS_DEFAULT;
        let mut relative = 0;
        // SAFETY: valid current pthread and writable output pointers.
        unsafe {
            libc::pthread_get_qos_class_np(libc::pthread_self(), &mut original, &mut relative);
        }
        if matches!(original, libc::qos_class_t::QOS_CLASS_UNSPECIFIED) {
            original = QOS_CLASS_DEFAULT;
        }
        Self {
            original,
            relative,
            active: None,
        }
    }

    pub fn update(&mut self, active: bool) {
        if self.active == Some(active) {
            return;
        }
        let class = if active {
            libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE
        } else {
            self.original
        };
        let relative = if active { 0 } else { self.relative };
        // SAFETY: only changes the caller's scheduling hint. Failure is
        // harmless (e.g. a thread opted out of the QoS system).
        unsafe {
            libc::pthread_set_qos_class_self_np(class, relative);
        }
        self.active = Some(active);
    }
}

#[cfg(target_os = "macos")]
impl Drop for UiPriority {
    fn drop(&mut self) {
        // SAFETY: restore this thread's original scheduling hint.
        unsafe {
            libc::pthread_set_qos_class_self_np(self.original, self.relative);
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub struct UiPriority;

#[cfg(not(target_os = "macos"))]
impl UiPriority {
    pub fn new() -> Self {
        Self
    }
    pub fn update(&mut self, _: bool) {}
}

/// Called first inside a worker, before any I/O or process launch.
pub fn worker() {
    #[cfg(target_os = "macos")]
    // SAFETY: applies a valid QoS hint to the calling worker only.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_DEFAULT, 0);
    }
}

/// Trace-only thread CPU clock, independent of scheduler wall-time stalls.
pub fn cpu_ns() -> u64 {
    #[cfg(target_os = "macos")]
    {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: a supported clock and a valid writable timespec.
        let result = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) };
        if result == 0 {
            return time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64;
        }
    }
    0 // Unsupported platforms report no CPU measurement.
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn priority_is_scoped_and_workers_shed_inherited_priority() {
        std::thread::spawn(|| {
            fn class() -> libc::qos_class_t {
                let mut class = libc::qos_class_t::QOS_CLASS_DEFAULT;
                // SAFETY: current pthread and valid output pointer.
                assert_eq!(
                    unsafe {
                        libc::pthread_get_qos_class_np(
                            libc::pthread_self(),
                            &mut class,
                            std::ptr::null_mut(),
                        )
                    },
                    0
                );
                class
            }
            let mut qos = UiPriority::new();
            let original = qos.original;
            qos.update(true);
            assert!(matches!(
                class(),
                libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE
            ));
            std::thread::spawn(|| {
                worker();
                assert!(matches!(class(), libc::qos_class_t::QOS_CLASS_DEFAULT));
            })
            .join()
            .unwrap();
            drop(qos);
            assert_eq!(class() as u32, original as u32);
            assert!(cpu_ns() > 0);
        })
        .join()
        .unwrap();
    }
}
