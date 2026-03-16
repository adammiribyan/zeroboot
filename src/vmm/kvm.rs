use anyhow::{Result, bail};
use kvm_bindings::*;
use kvm_bindings::CpuId;
use kvm_ioctls::{Kvm, VcpuExit, VmFd, VcpuFd};
use std::ptr;
use std::time::{Duration, Instant};

use super::serial::Serial;

extern "C" fn noop_signal_handler(_: libc::c_int) {}

const COM1_PORT: u16 = 0x3f8;
const COM1_PORT_END: u16 = 0x3ff;
const COM1_IRQ: u32 = 4;

pub struct VmSnapshot {
    pub regs: kvm_regs,
    pub sregs: kvm_sregs,
    pub msrs: Vec<kvm_msr_entry>,
    pub lapic: kvm_lapic_state,
    pub ioapic_redirtbl: [u64; 24],
    pub xcrs: kvm_xcrs,
    pub xsave: kvm_xsave,
    pub cpuid_entries: Vec<kvm_cpuid_entry2>,
    pub mem_size: usize,
}


pub struct ForkedVm {
    pub vm_fd: VmFd,
    pub vcpu_fd: VcpuFd,
    pub mem_ptr: *mut u8,
    pub mem_size: usize,
    pub serial: Serial,
    pub fork_time_us: f64,
    _kvm: Kvm,
}


impl ForkedVm {
    pub fn fork_cow(snapshot: &VmSnapshot, memfd: i32) -> Result<Self> {
        let start = Instant::now();

        let kvm = Kvm::new()?;
        let vm_fd = kvm.create_vm()?;
        vm_fd.create_irq_chip()?;
        vm_fd.create_pit2(kvm_pit_config::default())?;

        // Restore IOAPIC redirect table from Firecracker snapshot.
        // Use get_irqchip to get KVM's initialized IOAPIC state, then overwrite
        // just the redirect table entries to match the guest kernel's configuration.
        {
            let mut irqchip = kvm_irqchip::default();
            irqchip.chip_id = KVM_IRQCHIP_IOAPIC;
            vm_fd.get_irqchip(&mut irqchip)
                .map_err(|e| anyhow::anyhow!("get_irqchip(IOAPIC): {}", e))?;
            unsafe {
                for i in 0..24 {
                    irqchip.chip.ioapic.redirtbl[i].bits = snapshot.ioapic_redirtbl[i];
                }
            }
            vm_fd.set_irqchip(&irqchip)
                .map_err(|e| anyhow::anyhow!("set_irqchip(IOAPIC): {}", e))?;
        }

        let fork_mem = unsafe {
            libc::mmap(
                ptr::null_mut(), snapshot.mem_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_NORESERVE, memfd, 0,
            )
        };
        if fork_mem == libc::MAP_FAILED {
            bail!("mmap failed: {}", std::io::Error::last_os_error());
        }

        unsafe {
            vm_fd.set_user_memory_region(kvm_userspace_memory_region {
                slot: 0, guest_phys_addr: 0,
                memory_size: snapshot.mem_size as u64,
                userspace_addr: fork_mem as u64, flags: 0,
            })?;
        }

        let vcpu_fd = vm_fd.create_vcpu(0)?;

        // Use Firecracker's exact CPUID from the snapshot, not host defaults.
        // This ensures KVM interprets XSAVE state correctly and the guest sees
        // the same CPU features it saw at boot (matching XSAVE layout, AVX, etc.).
        if !snapshot.cpuid_entries.is_empty() {
            let cpuid = CpuId::from_entries(&snapshot.cpuid_entries)
                .map_err(|e| anyhow::anyhow!("CpuId::from_entries: {:?}", e))?;
            vcpu_fd.set_cpuid2(&cpuid)?;
        } else {
            // Fallback to host CPUID if snapshot has none (shouldn't happen)
            let cpuid = kvm.get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)?;
            vcpu_fd.set_cpuid2(&cpuid)?;
        }

        vcpu_fd.set_sregs(&snapshot.sregs)?;

        // Restore XCRS (must be after sregs — CR4.OSXSAVE must be set first)
        if snapshot.xcrs.nr_xcrs > 0 {
            vcpu_fd.set_xcrs(&snapshot.xcrs)
                .map_err(|e| anyhow::anyhow!("set_xcrs: {}", e))?;
        }

        // Restore XSAVE (FPU/SSE/AVX state — must be after XCRS)
        vcpu_fd.set_xsave(&snapshot.xsave)
            .map_err(|e| anyhow::anyhow!("set_xsave: {}", e))?;

        vcpu_fd.set_regs(&snapshot.regs)?;

        // Restore LAPIC
        let _ = vcpu_fd.set_lapic(&snapshot.lapic);

        // Restore MSRs (kvm-clock, syscall, etc.)
        if !snapshot.msrs.is_empty() {
            let mut msrs = Msrs::new(snapshot.msrs.len()).unwrap();
            for (i, entry) in snapshot.msrs.iter().enumerate() {
                let mut e = *entry;
                if e.index == 0x4b564d00 { e.data |= 1; }
                msrs.as_mut_slice()[i] = e;
            }
            let _ = vcpu_fd.set_msrs(&msrs);
        }

        // MUST be last: set MP state to RUNNABLE so vCPU isn't stuck in HLT
        let _ = vcpu_fd.set_mp_state(kvm_mp_state { mp_state: 0 });

        Ok(Self {
            _kvm: kvm, vm_fd, vcpu_fd,
            mem_ptr: fork_mem as *mut u8, mem_size: snapshot.mem_size,
            serial: Serial::new(),
            fork_time_us: start.elapsed().as_secs_f64() * 1_000_000.0,
        })
    }

    pub fn inject_serial_irq(&self) -> Result<()> {
        self.vm_fd.set_irq_line(COM1_IRQ, true).map_err(|e| anyhow::anyhow!("{}", e))?;
        self.vm_fd.set_irq_line(COM1_IRQ, false).map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    }

    pub fn send_serial(&mut self, data: &[u8]) -> Result<()> {
        self.serial.queue_input(data);
        self.serial.set_ier_data_ready(true);
        self.inject_serial_irq()?;
        Ok(())
    }

    pub fn run_until_marker(&mut self, marker: &str, max_exits: u64) -> Result<String> {
        self.run_until_marker_timeout(marker, max_exits, None)
    }

    pub fn run_until_marker_timeout(&mut self, marker: &str, max_exits: u64, timeout: Option<Duration>) -> Result<String> {
        if self.serial.has_pending_input() {
            self.serial.set_ier_data_ready(true);
            let _ = self.inject_serial_irq();
        }

        let debug = std::env::var("ZEROBOOT_DEBUG").is_ok();
        let deadline = timeout.map(|d| Instant::now() + d);

        // Set up a per-thread timer to interrupt KVM_RUN via signal.
        // KVM_RUN blocks in kernel mode; the only way to break out is a signal
        // that causes the ioctl to return EINTR.
        let _timer_guard = if let Some(dur) = timeout {
            // Install no-op SIGALRM handler (SA_RESTART=false so ioctl gets EINTR)
            unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = noop_signal_handler as *const () as usize;
                sa.sa_flags = 0; // no SA_RESTART
                libc::sigaction(libc::SIGALRM, &sa, std::ptr::null_mut());
            }
            // Create a per-thread timer using CLOCK_THREAD_CPUTIME_ID won't work
            // across threads, so use a helper thread that sleeps then signals us.
            let thread_id = unsafe { libc::pthread_self() };
            let handle = std::thread::spawn(move || {
                std::thread::sleep(dur);
                unsafe { libc::pthread_kill(thread_id, libc::SIGALRM); }
            });
            Some(handle)
        } else {
            None
        };

        let mut exit_count: u64 = 0;
        let mut io_in_count: u64 = 0;
        let mut io_out_count: u64 = 0;
        let mut hlt_count: u64 = 0;
        let mut _last_io_in_offset: u16 = 0;
        let mut _last_io_in_val: u8 = 0;

        for _ in 0..max_exits {
            if let Some(dl) = deadline {
                if Instant::now() >= dl {
                    bail!("Execution timed out");
                }
            }
            exit_count += 1;
            match self.vcpu_fd.run() {
                Ok(exit) => match exit {
                    VcpuExit::IoOut(port, data) => {
                        if port >= COM1_PORT && port <= COM1_PORT_END {
                            io_out_count += 1;
                            let offset = port - COM1_PORT;
                            let _thri_before = self.serial.thri_enabled();
                            for &b in data { self.serial.write(offset, b); }
                            // Level-triggered THRI: keep IRQ line asserted while
                            // THRI is enabled and de-assert when disabled. This
                            // matches real UART behavior where THRE interrupt
                            // stays pending until acknowledged by disabling THRI.
                            if offset == 1 {
                                if self.serial.thri_enabled() {
                                    let _ = self.vm_fd.set_irq_line(COM1_IRQ, true);
                                } else {
                                    let _ = self.vm_fd.set_irq_line(COM1_IRQ, false);
                                }
                            }
                            if debug && offset == 0 && exit_count < 5000 {
                                let ch = data[0];
                                if ch >= 0x20 && ch < 0x7f {
                                    eprintln!("  [{}] IoOut chr='{}' out_len={}", exit_count, ch as char, self.serial.output.len());
                                } else {
                                    eprintln!("  [{}] IoOut byte=0x{:02x} out_len={}", exit_count, ch, self.serial.output.len());
                                }
                            } else if debug && exit_count < 5000 {
                                eprintln!("  [{}] IoOut reg={} val=0x{:02x}", exit_count, offset, data[0]);
                            }
                            if String::from_utf8_lossy(&self.serial.output).contains(marker) {
                                if debug { eprintln!("  MARKER FOUND at exit {}", exit_count); }
                                return Ok(String::from_utf8_lossy(&self.serial.output).to_string());
                            }
                        }
                    }
                    VcpuExit::IoIn(port, data) => {
                        if port >= COM1_PORT && port <= COM1_PORT_END {
                            let offset = port - COM1_PORT;
                            data[0] = self.serial.read(offset);
                            io_in_count += 1;
                            _last_io_in_offset = offset;
                            _last_io_in_val = data[0];
                            if debug && exit_count < 5000 {
                                if offset == 0 {
                                    let ch = data[0];
                                    if ch >= 0x20 && ch < 0x7f {
                                        eprintln!("  [{}] IoIn  RX chr='{}' pending={}", exit_count, ch as char, self.serial.input_len());
                                    } else {
                                        eprintln!("  [{}] IoIn  RX byte=0x{:02x} pending={}", exit_count, ch, self.serial.input_len());
                                    }
                                } else if offset == 2 {
                                    eprintln!("  [{}] IoIn  IIR=0x{:02x}", exit_count, data[0]);
                                } else if offset == 5 {
                                    eprintln!("  [{}] IoIn  LSR=0x{:02x}", exit_count, data[0]);
                                }
                            }
                        } else {
                            data[0] = 0xff;
                        }
                    }
                    VcpuExit::Hlt => {
                        hlt_count += 1;
                        if debug { eprintln!("  [{}] HLT pending={}", exit_count, self.serial.has_pending_input()); }
                        if self.serial.has_pending_input() {
                            let _ = self.inject_serial_irq();
                        } else {
                            if debug { eprintln!("  DONE: exits={} io_in={} io_out={} hlt={}", exit_count, io_in_count, io_out_count, hlt_count); }
                            return Ok(String::from_utf8_lossy(&self.serial.output).to_string());
                        }
                    }
                    VcpuExit::Shutdown => {
                        return Ok(String::from_utf8_lossy(&self.serial.output).to_string());
                    }
                    VcpuExit::MmioRead(_, data) => { for b in data.iter_mut() { *b = 0xff; } }
                    VcpuExit::MmioWrite(_, _) => {}
                    VcpuExit::InternalError => bail!("KVM internal error"),
                    _ => {}
                },
                Err(e) => {
                    if e.errno() == libc::EAGAIN { continue; }
                    if e.errno() == libc::EINTR {
                        // Signal interrupted KVM_RUN — check if we've timed out
                        if let Some(dl) = deadline {
                            if Instant::now() >= dl {
                                bail!("Execution timed out");
                            }
                        }
                        continue;
                    }
                    bail!("KVM run: {} (errno {})", e, e.errno());
                }
            }
        }
        Ok(String::from_utf8_lossy(&self.serial.output).to_string())
    }
}

impl Drop for ForkedVm {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.mem_ptr as *mut libc::c_void, self.mem_size); }
    }
}

pub fn create_snapshot_memfd(mem_ptr: *const u8, mem_size: usize) -> Result<i32> {
    let name = std::ffi::CString::new("zeroboot-snapshot").unwrap();
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 { bail!("memfd_create failed"); }
    if unsafe { libc::ftruncate(fd, mem_size as i64) } < 0 {
        unsafe { libc::close(fd); }
        bail!("ftruncate failed");
    }
    let dst = unsafe {
        libc::mmap(ptr::null_mut(), mem_size, libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED, fd, 0)
    };
    if dst == libc::MAP_FAILED {
        unsafe { libc::close(fd); }
        bail!("mmap failed");
    }
    unsafe {
        ptr::copy_nonoverlapping(mem_ptr, dst as *mut u8, mem_size);
        libc::munmap(dst, mem_size);
    }
    Ok(fd)
}
