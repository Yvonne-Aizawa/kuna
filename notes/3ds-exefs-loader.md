# Nintendo 3DS ExeFS loader (proposal)

Nintendo 3DS titles run on an ARM11 (ARMv6K, `ARM:LE:32:v6:default`). The main
program ships as the ExeFS `.code` file, a flat image whose segment layout lives in
the NCCH extended header. Kuna has no loader for it, so `code.bin` can only be
loaded with `--raw-image`.

## Measured with `--raw-image` (Pokémon X `code.bin`, base 0x100000)

- Function discovery by direct calls finds 11,930 functions; vtable and callback
  targets are not seeded.
- Literal-pool loads are not folded, so globals print as `*dat_10ec74` (the pool
  slot) instead of the real address `dat_5d0e70`.
- Asserting `readonly 0x100000+0x43c000` and `readonly 0x53c000+0x6a000` by hand
  fixes the globals. `.data` stays unfolded when left out of the assertions.
- BSS is not mapped, `svc N` prints as `software_interrupt(N)`, and `--isa` is one
  setting for the whole image.

## Proposed behaviour

1. Detect an ExeFS `.code` image from a sibling `exheader.bin` (or an `--exheader`
   path) and map it with the segment table: `.text` executable and read-only,
   `.rodata` read-only, `.data` writable, then a zero-filled BSS of the declared
   size. Base and ranges come from the exheader, not from flags.
2. Name the `svc` numbers with their libctru/Horizon names.
3. Seed function discovery from vtable-shaped pointer runs in `.rodata`/`.data`.
4. Mark ARM versus Thumb per function from the interworking bit instead of one
   image-wide `--isa`.

Out of scope: `.cia`/`.cci`/`.3dsx` container parsing, `.cro` modules, encrypted
images, SDK library naming.

## Shape

- Loader in `kuna-analysis/src/loader/`, owned by `docs/spec/01-program-prep.md`
  §1.2/§1.3.
- Anything that changes emitted C (svc names, vtable seeding, per-function ISA)
  gets its own `settableTable` row in `phases.toml` and a handler in
  `kuna_option_dispatch.rs`, with the catalog counts bumped. Segment mapping only
  applies when an exheader is supplied, so existing inputs are unchanged.
- Stage test `tests/stages/` with a synthetic ExeFS image and exheader: option
  off shows `dat_<pool>` globals, default shows the resolved ones.
- Speed: measured on the Pokémon X image against `main` before the PR leaves draft;
  the datatest corpus must not move.

## Open questions

- Accept the `.code` file plus a separate exheader, or also read a raw ExeFS
  container so users do not have to extract it first?
- Should `svc` naming be a shared table usable by other Horizon-family targets?
