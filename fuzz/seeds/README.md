# Fuzz seeds

Inputs the two fuzz targets start from, committed on purpose. A fuzzer that starts from an empty
corpus spends its first minutes finding a boot sector a parse accepts, so it never reaches the tree
walk or the writer within a short run. Pass a directory here as a read-only corpus after the
target's own scratch corpus:

```sh
cargo fuzz run file_allocation_table_boot_sector fuzz/corpus/boot_sector \
    fuzz/seeds/file_allocation_table_boot_sector tests/fixtures
cargo fuzz run file_allocation_table_writer fuzz/corpus/writer fuzz/seeds/file_allocation_table_writer
```

| Target | Seeds | What they are |
|---|---|---|
| `file_allocation_table_boot_sector` | `tests/fixtures/` and `file_allocation_table_boot_sector/` | six boot sectors that dosfstools' `mkfs.fat` formatted (FAT12, FAT16 and FAT32, at 512 and 4096 bytes), and `small-tree.img`, below |
| `file_allocation_table_writer` | `file_allocation_table_writer/` | one seed of each kind the target decodes: `small-tree.img` behind the hostile-image byte, and a short script of operations behind the formatted-volume byte |

## `file_allocation_table_boot_sector/small-tree.img`

A 6 KiB FAT12 volume of eight one-sector clusters, the smallest that reaches every path of the
target's tree walk. It holds the volume label, a long name on a directory, a directory inside it, a
file chained over three clusters, a free slot and two free clusters. `make-fat12-tree.py` places
every byte by the specification's layout and rebuilds the file byte for byte, and `fsck.fat -n`
accepts it. The script sits beside the seed directories, not in one, so the fuzzer never reads it
as an input.
