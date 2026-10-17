# Nintendo 3DS CRO/CRS loader (proposal)

A `.cro` is a 3DS dynamic module (a `.crs` is the static-module table the game
loads first). Kuna does not recognise either ("Unknown file magic"), so a CRO has
to be cut up by hand and loaded as a raw image. That loses everything the module
carries about itself: segment permissions, the names it exports, the imports its
calls go through, and the relocations RO applies when it loads it.

## What a CRO holds (checked on `DllDialogCommon.cro`, 12 KiB)

Layout is from 3dbrew's CRO0 page; the numbers are from parsing the file.

- Magic `CRO0` at 0x80 (`FIXD` once RO has fixed it); name `DllDialogCommon`.
- 6 segment records, three in use: `.text` file 0x180 size 0xd08, `.rodata` 0x1000
  size 0x2c4, `.data` 0x28e0 size 0x14; `.bss` size 0x20 in the header.
- 1 named export (`nnroControlObject_`), no indexed exports.
- 1 import module (`|static|`, i.e. the game's own `code.bin`), 0x39 anonymous
  imports (each a segment offset in the static module), 62 import relocations.
- 76 internal relocations, all `R_ARM_ABS32`: the word at a segment offset gets
  the address of another segment plus an addend.

On disk the patched words are zero. The jump table of the switch at file 0x318 is
the case in point:

```
0x318  ldrcc pc,[pc,r0,lsl #2]      ; 0x379ff100
0x31c  b     default
0x320  0 0 0 0 0                    ; the table, zero on disk
```

and the internal relocations list those words with addends 436, 596, 616, 644 and
688 into segment 0. Today kuna prints `*(base + state*4 + 800)()` and the case
bodies fall out as unreachable.

## Proposed behaviour

1. Recognise `CRO0`/`FIXD` (and a CRS) by magic and map it: each segment at
   `base + file offset` with its type's permissions (`.text` executable,
   `.rodata` read-only, `.data` writable), and `.bss` as zero-filled space after
   the file. `--base` sets the load address and defaults to 0, so addresses are
   file offsets and match existing disassemblies.
2. Name the module's exports as function or data symbols, so
   `nnroControlObject_` appears instead of `sub_c580`.
3. Apply the relocations to the mapped bytes the way RO does: internal ABS32 to
   `target segment base + addend` (and `REL32`, `PREL31`, `CALL`, `JUMP24`,
   `THM_PC22` where present), so the zero table words become code addresses and the
   existing switch recovery sees a real table.
4. Give each import a synthetic external symbol in a block after the module
   (`import_<module>_<segment offset>`), write its address at the patch sites, and
   mark it as an external function, so a call through a patched pointer prints a
   name. The anonymous imports name no symbol, only an address in the static
   module.
5. Optional follow-up, not in the first change: `--static-module code.bin` to
   resolve each anonymous import to its real address in the game, so cross-module
   calls carry the names `code.bin` already has.

Out of scope: encrypted or compressed modules, CRS-driven cross-module linking
beyond item 5, the export trie, the `OnLoad`/`OnExit` hooks beyond naming them.

## Shape

- Loader in `kuna-analysis/src/loader/`, owned by `docs/spec/01-program-prep.md`
  §1.2/§1.3, parsed by hand like the TE loader (`loadimage_te.rs`) rather than
  through the `object` crate, which does not know the format.
- Applying relocations changes emitted C, so it ships behind an option
  `crorelocs on|off` (default on for CRO input only, which cannot be loaded at all
  today); naming exports and mapping segments are format support, not options.
- Tests: a stage case cannot hold a CRO, so a CLI test builds a small synthetic CRO
  (one text segment, a jump table patched by three ABS32 relocations, one export,
  one anonymous import) and checks the switch recovers with relocations on and
  stays a `jump-as-call` with the option off, plus unit tests for the header and
  relocation parsers. Speed: measured on `DllDialogCommon.cro` and the largest CRO
  in the game before the PR leaves draft.

## Open questions

- Default base: 0 (file offsets, matches existing disassemblies) or a typical
  RO load address?
- Is `--static-module` worth its own change, given that it needs `code.bin` and its
  `exheader.bin` together?
- ARM `ldrcc pc,[pc,r0,lsl #2]` plus `cmp r0,#N` is a switch idiom kuna may not
  recover even with a populated table; the first change should check that and say so.
