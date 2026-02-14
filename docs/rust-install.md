### Install gcc
```bash
sudo apt install make
sudo apt install gcc-13 make
sudo apt install gcc
```

### Install rust
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

```
### Build 
``` bash
RUSTFLAGS="-Awarnings"  cargo build --release -q

```