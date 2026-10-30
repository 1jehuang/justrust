set -e
# A registry of this machine's own, with one crate in it: nothing is fetched. It is a
# local-registry the `local` registry's source is replaced with, as blaze reads no git index.
mkdir -p registry/index/3/d stage/dep-0.1.0
cp -r published/Cargo.toml published/src stage/dep-0.1.0/
tar -czf registry/dep-0.1.0.crate -C stage dep-0.1.0
sum=$(sha256sum registry/dep-0.1.0.crate | cut -d' ' -f1)
echo '{"name":"dep","vers":"0.1.0","deps":[],"cksum":"'$sum'","features":{},"yanked":false}' > registry/index/3/d/dep
rm -rf stage
sed "s|@HERE@|${BM_HERE:-$PWD}|" Cargo.toml.in > Cargo.toml
sed "s|@HERE@|${BM_HERE:-$PWD}|" .cargo/config.toml.in > .cargo/config.toml
