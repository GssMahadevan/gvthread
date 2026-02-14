## Instyall coredump apps
```bash
sudo apt install gdb
sudo apt install systemd-coredump
```
## List coredumps
```bash
coredumpctl list 
```

## Debug
```bash
# Option 1: Use coredumpctl directly (easiest)
coredumpctl gdb pid_as_listed_in_coredumpctl_list
# then at (gdb) prompt:
bt
bt full
info threads
thread apply all bt

# Option 2: If you want symbols, rebuild with debug info
# In Cargo.toml or profile:
# [profile.release]
# debug = 1
cargo build -p gvthread-httpd --release
# Reproduce the crash, then:
coredumpctl gdb
```
