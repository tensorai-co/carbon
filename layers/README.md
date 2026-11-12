# layers

The file system programs see, for any kernel: an image's layers, merged, with a machine's own writable layer on top.
It knows only what's true of any file: its kind, size, times, contents, names and links. What a file means to its
kernel (owner and mode to lambda, attributes to atlas) is that kernel's `M`, kept here but never read.

**Built.** Both kernels use it, and so do `images`, `machine` and the CLI, to build, freeze and copy files in and out.

## How a disk is kept

- **One tree in memory**, names and metadata. Each file's contents are in 64 KiB chunks, each a hole, bytes in memory,
  or a SHA-256 name in a `Store`, read back as needed.
- **An image's index** (`index.rs`) is the image's layers merged into one, on disk as fixed-width tables
  of inodes, entries, chunk names, strings and small files' contents, with every offset checked, so a bad index is
  refused rather than trusted.
- **A machine's log** (`log.rs`) is its writable layer: every change since its image, replayed when the disk opens.
  Names and metadata are logged as they change, contents when the disk syncs: `sync`, a program's `fsync` or
  `FlushFileBuffers`, every 16 MiB written, and when the machine ends. A record is length-prefixed and checksummed, so
  a crash leaves the disk as it was at the last whole record.
- **`freeze`** writes the whole tree as a new index: that's how a machine becomes an image.
- **Tar** (`tar.rs`): a disk filled from a tar, with hard links, long names and OCI whiteouts.
- **Case.** A directory can fold case, as Windows' do: names that differ only in case are one name, and keep the case
  they were given.
- **Open files.** A file held open (`hold`) outlives its name, as programs' nameless temporary files need; the last
  `release` lets it go.
- **Extended attributes.** Each file keeps a map of extended attributes (`get_xattr`, `set_xattr`, `list_xattrs`,
  `remove_xattr`), logged like its metadata and replayed on reopen. (Not yet written into a frozen index.)
- **Volatile trees.** `volatile_root` makes a directory in no other whose tree is never logged, saved or frozen, and
  `drop_tree` lets it go: a kernel's tmpfs. Nothing links or moves between it and the rest (`CrossFs`).
- **Watching.** An observer (`observe`) is told of each change as it's made: made, linked, removed (and whether that
  was the last name), moved (and what it replaced), written, metadata set. lambda's inotify is built on it; atlas's
  `ReadDirectoryChangesW` could be.
- A kernel's `M` is saved as four words (`Persist`).

## Testing

```sh
cargo test -p layers
```

Extended attributes set, replaced, removed and replayed from the log; a volatile tree never reaching the log, nothing crossing out of it, and a file held open outliving it; an observer
told of each change in order, with the tree as it is after; writing and removing, folding case (and keeping it through an index), an open file outliving its name; a disk that
keeps its files when reopened, what wasn't synced lost cleanly after a crash, holes, files cut then grown, ranges zeroed in place (`fallocate`'s punched holes), freezing and
reopening, and a bad index refused.

## To do

- [ ] **Compacting a log in place.** A long-running machine's log only shrinks when it's frozen (`carbon freeze`, then
      a machine on the new image). Compacting it while the machine runs would need its files' numbers kept.
- [ ] **Sharing a running machine's disk**, for forking a machine (`sdk`'s `fork`): a second machine on the same
      index and a copy of the log.
- [ ] Files' permissions are kept in each kernel's `M` and not checked yet: that's lambda's to do, with credentials.
