set -e
# A registry of this machine's own, with one crate in it: nothing is fetched.
mkdir -p registry/index/3/d registry/crates stage/dep-0.1.0
cp -r published/Cargo.toml published/src stage/dep-0.1.0/
tar -czf registry/crates/dep-0.1.0.crate -C stage dep-0.1.0
sum=$(sha256sum registry/crates/dep-0.1.0.crate | cut -d' ' -f1)
echo '{"name":"dep","vers":"0.1.0","deps":[],"cksum":"'$sum'","features":{},"yanked":false}' > registry/index/3/d/dep
rm -rf stage
sed "s|@HERE@|${BM_HERE:-$PWD}|" registry/index/config.json.in > registry/index/config.json
(cd registry/index && git init -q && git add -A && git -c user.name=t -c user.email=t@t commit -qm index)
printf '[registries.local]\nindex = "file://%s/registry/index"\n' "${BM_HERE:-$PWD}" > .cargo/config.toml
cargo generate-lockfile --quiet
cargo vendor --quiet vendor > /dev/null
sed "s|@HERE@|${BM_HERE:-$PWD}|" .cargo/config.toml.in > .cargo/config.toml
rm -rf registry/crates
