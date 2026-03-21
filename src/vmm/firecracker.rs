use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const FC_SOCKET_TIMEOUT: Duration = Duration::from_secs(5);

pub struct FirecrackerVm {
    process: Child,
    socket_path: String,
    snapshot_dir: String,
}

#[derive(Serialize)]
struct BootSource {
    kernel_image_path: String,
    boot_args: String,
}

#[derive(Serialize)]
struct Drive {
    drive_id: String,
    path_on_host: String,
    is_root_device: bool,
    is_read_only: bool,
}

#[derive(Serialize)]
struct MachineConfig {
    vcpu_count: u32,
    mem_size_mib: u32,
}

#[derive(Serialize)]
struct SnapshotCreate {
    snapshot_type: String,
    snapshot_path: String,
    mem_file_path: String,
}

#[derive(Serialize)]
struct VmAction {
    action_type: String,
}

impl FirecrackerVm {
    pub fn boot(
        kernel_path: &str,
        rootfs_path: &str,
        work_dir: &str,
        mem_mib: u32,
        init_path: &str,
    ) -> Result<Self> {
        let socket_path = format!("{}/firecracker.sock", work_dir);
        let snapshot_dir = format!("{}/snapshot", work_dir);

        // Clean up
        let _ = std::fs::remove_file(&socket_path);
        std::fs::create_dir_all(&snapshot_dir)?;

        // Start Firecracker
        eprintln!("Starting Firecracker...");
        let process = Command::new("firecracker")
            .args(["--api-sock", &socket_path])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to start Firecracker")?;

        // Wait for socket
        let start = Instant::now();
        while !Path::new(&socket_path).exists() {
            if start.elapsed() > Duration::from_secs(5) {
                bail!("Firecracker socket didn't appear");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(50));

        let vm = Self {
            process,
            socket_path,
            snapshot_dir,
        };

        // Configure machine
        vm.api_put(
            "/machine-config",
            &MachineConfig {
                vcpu_count: 1,
                mem_size_mib: mem_mib,
            },
        )?;

        // Mask AVX2 from CPUID so no guest code selects AVX2 SIMD dispatch.
        // AVX2 dispatch paths hang after VM fork due to incomplete vCPU state
        // restore. By hiding AVX2 at the CPUID level, all packages (numpy,
        // pandas, etc.) use SSE4.2 baseline which works reliably after fork.
        vm.api_put(
            "/cpu-config",
            &serde_json::json!({
                "cpuid_modifiers": [{
                    "leaf": "0x7",
                    "subleaf": "0x0",
                    "flags": 0,
                    "modifiers": [{
                        "register": "ebx",
                        "bitmap": "0bxxxxxxxxxxxxxxxxxxxxxxxxxx0xxxxx"
                    }]
                }]
            }),
        )?;

        // Set boot source
        vm.api_put(
            "/boot-source",
            &BootSource {
                kernel_image_path: kernel_path.to_string(),
                boot_args: format!(
                    "console=ttyS0 reboot=k panic=1 pci=off random.trust_cpu=on init={}",
                    init_path
                ),
            },
        )?;

        // Add rootfs drive
        vm.api_put(
            "/drives/rootfs",
            &Drive {
                drive_id: "rootfs".to_string(),
                path_on_host: rootfs_path.to_string(),
                is_root_device: true,
                is_read_only: false,
            },
        )?;

        // Start the VM
        vm.api_put(
            "/actions",
            &VmAction {
                action_type: "InstanceStart".to_string(),
            },
        )?;

        eprintln!("Firecracker VM started");
        Ok(vm)
    }

    /// Pause the VM and create a snapshot
    pub fn snapshot(&mut self) -> Result<(String, String)> {
        let snapshot_path = format!("{}/vmstate", self.snapshot_dir);
        let mem_path = format!("{}/mem", self.snapshot_dir);

        // Pause the VM
        eprintln!("Pausing VM...");
        self.api_patch("/vm", &serde_json::json!({"state": "Paused"}))?;

        // Create snapshot
        eprintln!("Creating snapshot...");
        self.api_put(
            "/snapshot/create",
            &SnapshotCreate {
                snapshot_type: "Full".to_string(),
                snapshot_path: snapshot_path.clone(),
                mem_file_path: mem_path.clone(),
            },
        )?;

        // Wait for files
        std::thread::sleep(Duration::from_millis(500));

        if !Path::new(&snapshot_path).exists() {
            bail!("Snapshot state file not created");
        }
        if !Path::new(&mem_path).exists() {
            bail!("Snapshot memory file not created");
        }

        let mem_size = std::fs::metadata(&mem_path)?.len();
        eprintln!(
            "Snapshot created: state={}B, mem={}MB",
            std::fs::metadata(&snapshot_path)?.len(),
            mem_size / 1024 / 1024
        );

        Ok((snapshot_path, mem_path))
    }

    fn api_put<T: Serialize>(&self, path: &str, body: &T) -> Result<String> {
        self.api_request("PUT", path, body)
    }

    fn api_patch<T: Serialize>(&self, path: &str, body: &T) -> Result<String> {
        self.api_request("PATCH", path, body)
    }

    fn api_request<T: Serialize>(&self, method: &str, path: &str, body: &T) -> Result<String> {
        let body_json = serde_json::to_string(body)?;
        let request = format!(
            "{} {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            method, path, body_json.len(), body_json
        );

        let mut stream = UnixStream::connect(&self.socket_path)
            .with_context(|| format!("Connect to Firecracker socket at {}", self.socket_path))?;
        stream.set_read_timeout(Some(FC_SOCKET_TIMEOUT))?;
        stream.set_write_timeout(Some(FC_SOCKET_TIMEOUT))?;

        stream.write_all(request.as_bytes())?;
        stream.flush()?;

        let mut response = vec![0u8; 4096];
        let n = stream.read(&mut response)?;
        let resp = String::from_utf8_lossy(&response[..n]).to_string();

        if !resp.contains("204") && !resp.contains("200") {
            bail!("Firecracker API error on {} {}: {}", method, path, resp);
        }

        Ok(resp)
    }

    pub fn kill(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

impl Drop for FirecrackerVm {
    fn drop(&mut self) {
        self.kill();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Boot a Firecracker VM, wait for it to be ready, then snapshot it.
/// Returns the paths to the snapshot files.
pub fn create_template_snapshot(
    kernel_path: &str,
    rootfs_path: &str,
    work_dir: &str,
    mem_mib: u32,
    wait_secs: u64,
    init_path: &str,
) -> Result<(String, String, u32)> {
    let mut vm = FirecrackerVm::boot(kernel_path, rootfs_path, work_dir, mem_mib, init_path)?;

    // Wait for the guest to boot and become ready
    eprintln!("Waiting {}s for guest to boot...", wait_secs);
    std::thread::sleep(Duration::from_secs(wait_secs));

    // Take snapshot
    let (state_path, mem_path) = vm.snapshot()?;

    Ok((state_path, mem_path, mem_mib))
}
