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
- **`freeze`** writes the whole tree as a new index: that's how a machine becomes an image. The chunks the index
  names are shared (`Store::share`) from then on, so they're never freed with the machine's files.
- **Deleting frees space.** A machine's store keeps the chunks the machine wrote as its own, apart from those images
  share (`Store::own_chunks`). Once about 16 MiB of chunks have been dropped (files removed, cut or written over), the
  disk syncs and frees each of its own chunks that no file names (`free_unnamed`): the log is synced first, so a chunk
  only goes once the records that stopped naming it survive a crash. On a full disk, where changed files can't be
  saved, the chunks they had when last saved are kept and the rest still go. A file held open keeps its chunks until
  it's let go. A file the store refuses part of (`NoSpace`) keeps all its changes in memory and is saved on the next
  sync.
- **Space.** `space` is the size and free space of the disk the store is on, less what's written but not yet saved,
  for `df`; none for a store in memory.
- **Tar** (`tar.rs`), both ways: a disk filled from a tar, with hard links, long names, PAX numbers and OCI
  whiteouts; and a disk written as one (`export`), whole or as what differs from another disk (a layer, with
  whiteouts for what's gone). Two files are compared by their chunks' names when both are in the store, so an
  unchanged file is never read; each folder's names go in their order, so the same disks always make the same bytes.
- **Case.** A directory can fold case, as Windows' do: names that differ only in case are one name, and keep the case
  they were given.
- **Open files.** A file held open (`hold`) outlives its name, as programs' nameless temporary files need; the last
  `release` lets it go.
- **Extended attributes.** Each file keeps a map of extended attributes (`get_xattr`, `set_xattr`, `list_xattrs`,
  `remove_xattr`), logged like its metadata and replayed on reopen. (Not yet written into a frozen index.)
- **External trees** (`external.rs`). `attach` makes a folder from elsewhere (a host's, a volume's) a tree of the
  disk's, through `External`: its files aren't kept in the tree, log or index; each call on one (lookup, read, write,
  make, link, rename, remove, metadata) is made on the folder by the file's path as it comes, so what changes there
  is seen at once. A file gets a number (from 2^48 up) when it's first found, and keeps it while its path does, a
  move included. Its `M` comes from the folder (`Mapping`): kept with each file by one that keeps it, else made from
  the host's mode, with what's set kept in memory. Nothing links or moves between it and the rest (`CrossFs`); one
  attached read-only refuses changes (`ReadOnly`). `meta` and `read_link` give owned values, so no reference into a
  tree that's read as it's asked is ever handed out.
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
reopening, and a bad index refused. Removing a file frees the machine's own chunks it had, and a crash after leaves the
other files whole; a file held open keeps its chunks until it's let go; chunks written over are freed once the log
names the new ones; a full disk frees what it can, keeps what the log still names, and saves the rest once there's
room; freezing shares what the image names, so it's never freed. A tree written as a tar and read back is the same tree (hard links, devices, FIFOs,
long paths and big owners through PAX); a layer of what changed, applied to its base, makes the new tree, and a tree
differs from itself in nothing.

## To do

- [ ] **Compacting a log in place.** A long-running machine's log only shrinks when it's frozen (`carbon freeze`, then
      a machine on the new image). Compacting it while the machine runs would need its files' numbers kept. (The
      chunks deleted files had are freed already; only the log's records stay.)
- [ ] **Sharing a running machine's disk**, for forking a machine (`sdk`'s `fork`): a second machine on the same
      index and a copy of the log.
- [x] Files' permissions are kept in each kernel's `M`; lambda checks them.
- [ ] Extended attributes in a frozen index, and so in an exported layer (`security.capability`, ACLs).
