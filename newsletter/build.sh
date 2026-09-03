#!/bin/bash
# Build lindfors-newsletter for the host it runs on: mail.lindfors.no, Alpine on aarch64,
# musl libc.
#
# No container and no cross C toolchain, which is the payoff for carrying no TLS stack
# (see Cargo.toml). With rustls in the tree this needed `cross` and a running docker,
# because rustls' crypto provider is partly C and `rustup target add` supplies only the
# Rust half. Everything this binary talks to is a port on that same machine.

set -e

cd "$(dirname "${BASH_SOURCE[0]}")"

TARGET="aarch64-unknown-linux-musl"
BIN="target/$TARGET/release/lindfors-newsletter"

if ! rustup target list --installed | grep -qx "$TARGET"; then
    echo "Adding $TARGET..."
    rustup target add "$TARGET"
fi

echo "Building for $TARGET..."
cargo build --release --target "$TARGET"

# Not stripped by the profile: a stripped binary is a stack trace nobody can read, and
# this is a few megabytes either way. Strip here if the size ever matters.
echo
echo "Built: $BIN"
ls -lh "$BIN" | awk '{print "  size: " $5}'
file "$BIN" 2>/dev/null | sed 's/^/  /' || true

cat <<'EOF'

Copy it over, then on the host as root:

  addgroup -S lindfors-newsletter
  adduser -S -D -H -s /sbin/nologin -G lindfors-newsletter lindfors-newsletter
  install -d -o lindfors-newsletter -g lindfors-newsletter /opt/lindfors-newsletter
  install -m 755 lindfors-newsletter /opt/lindfors-newsletter/
  install -m 755 lindfors-newsletter.openrc /etc/init.d/lindfors-newsletter
  install -m 600 env.example /etc/lindfors-newsletter.env    # then fill it in
  rc-update add lindfors-newsletter default && rc-service lindfors-newsletter start
EOF
