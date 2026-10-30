set -e
# A registry of this machine's own with two versions of one crate: nothing is fetched. It is
# a local-registry the `local` registry's source is replaced with, as blaze reads no git index.
mkdir -p registry/index/3/d
for v in 0.1.0 0.1.1; do
  mkdir -p stage/dep-$v/src
  printf '[package]\nname = "dep"\nversion = "%s"\nedition = "2021"\ndescription = "x"\nlicense = "MIT"\n' $v > stage/dep-$v/Cargo.toml
  printf 'pub fn version() -> &'"'"'static str {\n    env!("CARGO_PKG_VERSION")\n}\n' > stage/dep-$v/src/lib.rs
  tar -czf registry/dep-$v.crate -C stage dep-$v
  sum=$(sha256sum registry/dep-$v.crate | cut -d' ' -f1)
  echo '{"name":"dep","vers":"'$v'","deps":[],"cksum":"'$sum'","features":{},"yanked":false}' >> registry/index/3/d/dep
done
rm -rf stage
mkdir -p .cargo
index="file://${BM_HERE:-$PWD}/registry/index"
printf '[registries.local]\nindex = "%s"\n\n[source.local-index]\nregistry = "%s"\nreplace-with = "local-files"\n\n[source.local-files]\nlocal-registry = "registry"\n' "$index" "$index" > .cargo/config.toml
cargo generate-lockfile --quiet
cargo update --quiet -p dep --precise 0.1.0
