# Swifty hashing + ordering spec (byte-perfect with SwiftyBackend C#)

This document defines the **canonical** ways SwiftyBackend (C#) computes checksums and orders inputs, and the **exact Rust rules** required to match it byte-for-byte.

All digest strings are rendered as **uppercase hex** (`0-9A-F`) with **no separators**.

---

## 0) Canonical primitives (C#)

### 0.1 Encoding
When hashing strings, SwiftyBackend uses:

- `Encoding.UTF8.GetBytes(string)`  
- No BOM, no null terminators, no separators between concatenated elements.

### 0.2 Hex formatting
SwiftyBackend ultimately turns `byte[]` digests into uppercase hex using `Utilities.ByteArrayToHexString(...)`.

**Canonical output:**
- Uppercase hex
- No `0x`
- No hyphens
- Length is 32 for MD5, 40 for SHA1

### 0.3 Incremental hashing semantics
SwiftyBackend uses incremental hashing:

- `TransformBlock(bytes...)` for each segment
- `TransformFinalBlock(new byte[0], 0, 0)` at the end

This is equivalent to hashing the concatenation of all segments in-order.

---

## 1) Part checksum (MD5 of raw bytes)

### 1.1 Definition
For each part `p`:

```

PartChecksumHex = HEX_UPPER( MD5( RawPartBytes ) )

```

### 1.2 Regular file partitioning (SwiftyFile)
Regular files are split into fixed-size chunks:

- Chunk size: **5,000,000 bytes**
- Parts are contiguous, cover the whole file (last chunk may be smaller).
- Part `Start` is the byte offset in the file.
- Part `Length` is the number of bytes in that chunk.

**Part name (Path):**
Swifty uses a naming pattern:
```

{FileName}_{EndOffset}

```
Where:
- `FileName` is the filename only (no directory)
- `EndOffset` is cumulative end offset (`Start + Length`)

### 1.3 PBO partitioning (SwiftyPboFile)
`.pbo` files are partitioned structurally:

1) `$$HEADER$$`: bytes `[0 .. header_len)`
2) One part per entry payload (in header order, skipping the first dummy entry)
3) `$$END$$`: remaining bytes after last payload (often signature/trailer)

Each part checksum is MD5 of the exact byte range.

**Important:** entry names are taken from the PBO header strings. In C#, these strings are decoded with UTF-8 *replacement fallback* (lossy) via `Encoding.UTF8.GetString(...)`.

Rust must also decode lossily to match (`from_utf8_lossy`).

---

## 2) File checksum (MD5 of concatenated **part checksum strings**)

SwiftyBackend computes the full file checksum from part checksum **hex strings**, not the raw file bytes.

### 2.1 Definition
Given ordered parts `p0..pn`:

```

FileChecksumHex = HEX_UPPER(
MD5( UTF8(p0.PartChecksumHex) || UTF8(p1.PartChecksumHex) || ... || UTF8(pn.PartChecksumHex) )
)

```

### 2.2 C# canonical behavior (from SwiftyBackend/SwiftyFiles/SwiftyFile.cs)
SwiftyBackend does:

- For each part:
  - `Encoding.UTF8.GetBytes(part.Checksum)`   // part.Checksum is uppercase hex string
  - `TransformBlock(...)`
- Finalize to hex upper

This means **the hashed bytes are the UTF-8 bytes of the uppercase hex text**.

---

## 3) Mod checksum (MD5 aggregate over files, with Swifty sorting)

SwiftyBackend mod checksum (addon checksum) is computed over the set of files in the mod.

### 3.1 File sort key: `CleanPath`
SwiftyBackend defines a `CleanPath(string input)` used for ordering.

**Exact C# logic (from SwiftyBackend/SwiftyAddon.cs):**
- Iterate each character in the input path
- Skip characters equal to:
  - `Path.PathSeparator`
  - `Path.AltDirectorySeparatorChar`
  - `Path.DirectorySeparatorChar`
- Return `.ToLowerInvariant()` of the filtered string

**Practical note:**
SwiftyBackend is typically run on Windows, where:
- `Path.PathSeparator` = `;`
- `Path.DirectorySeparatorChar` = `\`
- `Path.AltDirectorySeparatorChar` = `/`

If SwiftyBackend runs on Linux/macOS, `Path.PathSeparator` differs (`:`), and `DirectorySeparatorChar` differs (`/`).
Rust must match the environment Swifty used to generate the canonical artifacts you want to reproduce.

### 3.2 Sort ordering: invariant + ordinal ignore case (unstable)
SwiftyBackend sorts the file list using `List.Sort(...)` with a comparison:

```

string.Compare(
CleanPath(x.RelativePath),
CleanPath(y.RelativePath),
CultureInfo.InvariantCulture,
CompareOptions.OrdinalIgnoreCase
)

```

**Key consequences**
1) The comparison is **ordinal** (code-unit based), not linguistic.
2) The sort is **unstable** (`List<T>.Sort` is not stable).
3) `CleanPath(...)` already lowercases with `ToLowerInvariant()`, but Swifty still compares with `OrdinalIgnoreCase`.

**Important implementation detail (observed mismatch + fix):**
- `OrdinalIgnoreCase` does **not** behave like a plain ordinal compare on already-lowercased strings.
- It effectively compares **upper-invariant UTF-16 code units** (e.g., `_` vs `s` differs from `_` vs `S`).
- For parity, build a sort key of `ToUpperInvariant(CleanPath(path))` and compare it by UTF-16 code units.
- This fixes cases like `addons\\ace_compat_rh_acc.pbo` vs `addons\\ace_compat_rhs_afrf3.pbo`, where
  ordinal compare orders `_` (U+005F) before `s` (U+0073), but `OrdinalIgnoreCase` compares `_` to `S` (U+0053),
  flipping the order and changing the mod checksum.

### 3.3 Hash input per file
For each file in the sorted list, SwiftyBackend appends to an MD5 hasher:

1) `UTF8(file.Checksum)` — where `file.Checksum` is the file checksum hex string
2) `UTF8(file.RelativePath.ToLowerInvariant().Replace('\\','/'))`

Then finalizes as uppercase hex.

### 3.4 Definition
Let `FilesSorted` be files sorted as in §3.2:

```

ModChecksumHex = HEX_UPPER(
MD5(
for f in FilesSorted:
UTF8(f.FileChecksumHex) ||
UTF8( f.RelativePath.ToLowerInvariant().Replace('\','/') )
)
)

```

---

## 4) Repo checksum (SHA1 aggregate over ticks + mod checksums)

Swifty repo checksum is SHA1 over concatenated strings, no delimiters.

### 4.1 Definition
```

RepoChecksumHex = HEX_UPPER(
SHA1(
UTF8(TicksDecimalString) ||
for m in RequiredMods (in list order): UTF8(m.ModChecksumHex) ||
for m in OptionalMods (in list order): UTF8(m.ModChecksumHex)
)
)

````

### 4.2 Ordering rules
- **RequiredMods order is preserved.** No sorting is applied during repo checksum generation.
- **OptionalMods order is preserved.** No sorting is applied.
- If your pipeline builds RequiredMods in a specific order (e.g., manifest order), that order becomes checksum-critical.

### 4.3 Ticks
Ticks are a decimal string of .NET ticks (100ns intervals since 0001-01-01).

---

## 5) Rust parity requirements (exactly matching SwiftyBackend)

This section specifies the Rust behaviors needed to reproduce the C# semantics **exactly**.

### 5.1 Lowercasing must match `ToLowerInvariant()` (Unicode-aware)
**Do not use `to_ascii_lowercase`.**

Use:

```rust
fn to_lower_invariant(s: &str) -> String {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}
````

This matches Unicode case mapping behavior used by `.ToLowerInvariant()`.

### 5.2 Sorting must match C# ordinal semantics (UTF-16 code units)

C# “ordinal” comparisons are performed over UTF-16 code units.
Rust `String` comparison is by Unicode scalar values, which can differ for non-BMP characters.

To match C# ordering, compare the UTF-16 sequences:

```rust
use std::cmp::Ordering;

fn cmp_utf16_ordinal(a: &str, b: &str) -> Ordering {
    let mut ia = a.encode_utf16();
    let mut ib = b.encode_utf16();
    loop {
        match (ia.next(), ib.next()) {
            (Some(x), Some(y)) => match x.cmp(&y) {
                Ordering::Equal => continue,
                ord => return ord,
            },
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => return Ordering::Equal,
        }
    }
}
```

Because Swifty sorts with an **unstable** algorithm (`List.Sort`), use Rust `sort_unstable_by(...)` to match that “mistake.”

### 5.3 `CleanPath` must use Swifty’s separators

Swifty uses `.NET Path.PathSeparator`, `.DirectorySeparatorChar`, `.AltDirectorySeparatorChar`.

If you are reproducing Windows-generated Swifty artifacts (typical SwiftyBackend usage), Rust should use:

* PathSeparator: `;`
* DirectorySeparatorChar: `\`
* AltDirectorySeparatorChar: `/`

Do not “auto-detect” your current Rust OS unless you intend to match Swifty running on that same OS.

Canonical Windows Swifty `CleanPath` in Rust:

```rust
fn clean_path_for_sort_windows(path: &str) -> String {
    let mut cleaned = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, ';' | '/' | '\\') {
            continue;
        }
        cleaned.push(c);
    }
    to_lower_invariant(&cleaned)
}
```

### 5.4 `RelativePath` canonicalization in the hash must match C#

Swifty hashes:

```
item.RelativePath.ToLowerInvariant().Replace('\\','/')
```

Rust equivalent:

```rust
fn path_for_checksum(path: &str) -> String {
    to_lower_invariant(path).replace('\\', "/")
}
```

### 5.5 PBO entry string decoding must be lossy

C# `Encoding.UTF8.GetString(bytes)` uses replacement fallback.
Rust must use `String::from_utf8_lossy`.

---

## 6) Drop-in Rust implementation (recommended canonical functions)

Use these exact functions in `libs/swifty_artifacts/src/checksum.rs`:

```rust
use std::cmp::Ordering;

#[inline]
fn to_lower_invariant(s: &str) -> String {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}

fn cmp_utf16_ordinal(a: &str, b: &str) -> Ordering {
    let mut ia = a.encode_utf16();
    let mut ib = b.encode_utf16();
    loop {
        match (ia.next(), ib.next()) {
            (Some(x), Some(y)) => match x.cmp(&y) {
                Ordering::Equal => continue,
                ord => return ord,
            },
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => return Ordering::Equal,
        }
    }
}

// Windows SwiftyBackend canonical CleanPath (typical Swifty artifacts):
fn clean_path_for_sort(path: &str) -> String {
    let mut cleaned = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, ';' | '/' | '\\') {
            continue;
        }
        cleaned.push(c);
    }
    to_lower_invariant(&cleaned)
}

// Matches: RelativePath.ToLowerInvariant().Replace('\\','/')
fn path_for_checksum(path: &str) -> String {
    to_lower_invariant(path).replace('\\', "/")
}
```

And sort like Swifty:

```rust
keyed.sort_unstable_by(|(ka, _), (kb, _)| cmp_utf16_ordinal(ka, kb));
```

---

## 7) Known “Rust gotchas” that break parity

These are common mismatches that cause checksum drift:

1. **ASCII-only lowercasing** (`to_ascii_lowercase`)
   → breaks `ToLowerInvariant()` for non-ASCII.

2. **Rust `String` ordering** for sort keys
   → differs from C# ordinal UTF-16 for non-BMP characters.

3. **Stable sorting**
   → differs from C# `List.Sort` when keys compare equal.

4. **Rejecting non-ASCII paths**
   → Swifty hashes them; Rust must not reject them for parity.

5. **Strict UTF-8 decoding for PBO strings**
→ C# is lossy; strict decoding changes names (and downstream hashing).
