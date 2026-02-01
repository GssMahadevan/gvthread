# Disable apport
sudo systemctl stop apport
sudo systemctl disable apport

# Set core pattern to current directory
echo "core.%e.%p" | sudo tee /proc/sys/kernel/core_pattern

# Enable core dumps
ulimit -c unlimited

# Run test
cargo build -p gvthread-playground
GVT_WORKERS=4 GVT_GVTHREADS=15000 GVT_YIELDS=3 GVT_SLEEP_MS=100 \
  ./target/debug/playground

# Debug core
gdb ./target/debug/playground core.*


# bt                      # backtrace
# bt full                 # with locals
# info threads            # list all threads
# thread apply all bt     # backtrace all threads
# frame N                 # select frame
# info registers          # show registers
# x/20x $rsp              # examine stack