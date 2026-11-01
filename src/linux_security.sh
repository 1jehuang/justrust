# Exposure reduction only, not a kernel fix. Keep aligned with justrust-cloud/src/linux_security.js.
install -d -m 755 /etc/justrust
cat > /etc/justrust/cve-2026-98432.nft <<'NFT'
table inet cve_2026_98432 {
  chain input {
    type filter hook input priority -310; policy accept;
    ip protocol tcp ip hdrlength > 5 counter drop
  }
}
NFT
chmod 644 /etc/justrust/cve-2026-98432.nft
cat > /usr/local/sbin/justrust-ipv4-tcp-guard <<'GUARD'
#!/bin/sh
set -eu
# One atomic transaction replaces only our own table, without duplicate rules.
# destroy is idempotent even on first boot, unlike delete.
{ printf '%s\n' 'destroy table inet cve_2026_98432'; cat /etc/justrust/cve-2026-98432.nft; } > /run/justrust-ipv4-tcp-guard.nft
/usr/sbin/nft --check --file /run/justrust-ipv4-tcp-guard.nft
/usr/sbin/nft --file /run/justrust-ipv4-tcp-guard.nft
/usr/sbin/nft list table inet cve_2026_98432
GUARD
chmod 755 /usr/local/sbin/justrust-ipv4-tcp-guard
cat > /etc/systemd/system/justrust-ipv4-tcp-guard.service <<'UNIT'
[Unit]
Description=Reduce CVE-2026-98432 exposure (not a kernel fix)
DefaultDependencies=no
After=local-fs.target nftables.service
Before=network-pre.target ssh.service ssh.socket
Wants=network-pre.target
[Service]
Type=oneshot
ExecStart=/usr/local/sbin/justrust-ipv4-tcp-guard
RemainAfterExit=yes
[Install]
WantedBy=multi-user.target
UNIT
# If the boot-time guard fails, do not expose the build SSH endpoint.
for unit in ssh.service ssh.socket; do
  install -d -m 755 "/etc/systemd/system/$unit.d"
  cat > "/etc/systemd/system/$unit.d/20-justrust-ipv4-tcp-guard.conf" <<'SSHUNIT'
[Unit]
Requires=justrust-ipv4-tcp-guard.service
After=justrust-ipv4-tcp-guard.service
SSHUNIT
done
systemctl daemon-reload
systemctl enable justrust-ipv4-tcp-guard.service
# Invoke directly too: an already-active oneshot must not hide missing rules.
/usr/local/sbin/justrust-ipv4-tcp-guard
systemctl start justrust-ipv4-tcp-guard.service
