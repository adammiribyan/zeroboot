#!/bin/bash
set -euo pipefail

# Deploy Zeroboot to remote servers.
#
# Usage:
#   SERVERS="host1 host2 host3" ./deploy/deploy.sh
#
# Environment:
#   SERVERS        Space-separated list of SSH hosts (required)
#   ZEROBOOT_BIN   Path to binary (default: target/release/zeroboot)
#   KERNEL         Path to kernel (default: guest/vmlinux-fc)
#   ROOTFS_PYTHON  Python rootfs (default: guest/rootfs-alpine-nosim.ext4)
#   ROOTFS_NODE    Node.js rootfs (default: guest/rootfs-node.ext4)

SERVERS="${SERVERS:-}"
ZEROBOOT_BIN="${ZEROBOOT_BIN:-target/release/zeroboot}"
KERNEL="${KERNEL:-guest/vmlinux-fc}"
ROOTFS_PYTHON="${ROOTFS_PYTHON:-guest/rootfs-alpine-nosim.ext4}"
ROOTFS_NODE="${ROOTFS_NODE:-guest/rootfs-node.ext4}"

if [ -z "$SERVERS" ]; then
    echo "Usage: SERVERS='host1 host2' ./deploy/deploy.sh"
    exit 1
fi

if [ ! -f "$ZEROBOOT_BIN" ]; then
    echo "Building release binary..."
    cargo build --release
fi

for server in $SERVERS; do
    echo "=== Deploying to $server ==="

    echo "  Setting up directories..."
    ssh "$server" "sudo mkdir -p /var/lib/zeroboot /etc/zeroboot && sudo chown zeroboot:zeroboot /var/lib/zeroboot"

    echo "  Copying binary..."
    scp "$ZEROBOOT_BIN" "$server":/tmp/zeroboot
    ssh "$server" "sudo mv /tmp/zeroboot /usr/local/bin/zeroboot && sudo chmod +x /usr/local/bin/zeroboot"

    echo "  Copying kernel and rootfs..."
    scp "$KERNEL" "$server":/tmp/vmlinux-fc
    ssh "$server" "sudo mv /tmp/vmlinux-fc /var/lib/zeroboot/"
    scp "$ROOTFS_PYTHON" "$server":/tmp/rootfs-python.ext4
    ssh "$server" "sudo mv /tmp/rootfs-python.ext4 /var/lib/zeroboot/"
    scp "$ROOTFS_NODE" "$server":/tmp/rootfs-node.ext4
    ssh "$server" "sudo mv /tmp/rootfs-node.ext4 /var/lib/zeroboot/"

    echo "  Creating templates..."
    ssh "$server" "cd /var/lib/zeroboot && sudo cp rootfs-python.ext4 python-rootfs.ext4 && sudo /usr/local/bin/zeroboot template vmlinux-fc python-rootfs.ext4 python 15 /init.py"
    ssh "$server" "cd /var/lib/zeroboot && sudo cp rootfs-node.ext4 node-rootfs.ext4 && sudo /usr/local/bin/zeroboot template vmlinux-fc node-rootfs.ext4 node 10 /init-node.sh"

    echo "  Installing service..."
    scp deploy/zeroboot.service "$server":/tmp/zeroboot.service
    ssh "$server" "sudo mv /tmp/zeroboot.service /etc/systemd/system/ && sudo systemctl daemon-reload && sudo systemctl enable zeroboot"

    echo "  Starting service..."
    ssh "$server" "sudo systemctl restart zeroboot"
    sleep 3

    echo "  Verifying..."
    if ssh "$server" "curl -sf localhost:8080/v1/health" | grep -q '"ok"'; then
        echo "  OK"
    else
        echo "  FAILED"
    fi

    echo ""
done

echo "Deploy complete."
