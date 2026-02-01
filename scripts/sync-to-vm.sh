#!/bin/bash
#
# Sync gvthread from Mac to Linux VM
#
# Usage (on Mac):
#   ./sync-to-vm.sh              # Uses default VM
#   ./sync-to-vm.sh user@host    # Specify VM
#   ./sync-to-vm.sh user@host build  # Sync and build
#
# Configuration:
#   Set GVTHREAD_VM environment variable or edit DEFAULT_VM below
#

DEFAULT_VM="${GVTHREAD_VM:-user@your-proxmox-vm}"
REMOTE_PATH="~/gvthread"

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

log_cmd() {
    echo -e "${BLUE}>>> $@${NC}"
}

# Parse arguments
VM="${1:-$DEFAULT_VM}"
ACTION="${2:-sync}"

if [ "$VM" = "user@your-proxmox-vm" ]; then
    echo "Error: Please configure your VM address."
    echo ""
    echo "Options:"
    echo "  1. Set environment variable:"
    echo "     export GVTHREAD_VM=user@your-vm-ip"
    echo ""
    echo "  2. Pass as argument:"
    echo "     $0 user@your-vm-ip"
    echo ""
    echo "  3. Edit DEFAULT_VM in this script"
    exit 1
fi

echo -e "${GREEN}Syncing to: $VM${NC}"
echo ""

# Rsync with common exclusions
log_cmd "rsync -avz --delete \\"
echo "    --exclude target/ \\"
echo "    --exclude .git/ \\"
echo "    --exclude '*.swp' \\"
echo "    --exclude '.DS_Store' \\"
echo "    $PROJECT_DIR/ $VM:$REMOTE_PATH/"

rsync -avz --delete \
    --exclude 'target/' \
    --exclude '.git/' \
    --exclude '*.swp' \
    --exclude '.DS_Store' \
    --exclude '.idea/' \
    --exclude '.vscode/' \
    "$PROJECT_DIR/" "$VM:$REMOTE_PATH/"

echo ""
echo -e "${GREEN}Sync complete!${NC}"

# Optional: build on remote
if [ "$ACTION" = "build" ] || [ "$2" = "build" ]; then
    echo ""
    log_cmd "ssh $VM 'cd $REMOTE_PATH && cargo build'"
    ssh "$VM" "cd $REMOTE_PATH && cargo build"
fi

if [ "$ACTION" = "test" ] || [ "$2" = "test" ]; then
    echo ""
    log_cmd "ssh $VM 'cd $REMOTE_PATH && cargo test'"
    ssh "$VM" "cd $REMOTE_PATH && cargo test"
fi

echo ""
echo "To build/test manually:"
echo "  ssh $VM"
echo "  cd $REMOTE_PATH"
echo "  cargo build"
