#!/usr/bin/env python3
"""Read members of a remote zip file with HTTP range requests, without downloading the whole zip.

  zip_range.py list URL                  print "size name" for every member
  zip_range.py get URL OUTDIR NAME...    write each member to OUTDIR/<basename> (zip CRC checked)

Standard library only. Used by fetch-corpus48.sh for a speaker subset of the 11 GB VCTK zip.
"""
import io
import os
import sys
import time
import urllib.error
import urllib.request
import zipfile


class HttpFile(io.RawIOBase):
    """A seekable read-only file over HTTP range requests, with one read-ahead block."""

    BLOCK = 256 << 10

    def __init__(self, url):
        self.url, self.pos = url, 0
        req = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(req, timeout=60) as r:
            if r.headers.get("Accept-Ranges") != "bytes":
                raise OSError(f"{url}: server does not accept range requests")
            self.size = int(r.headers["Content-Length"])
        self.buf_start, self.buf = 0, b""

    def readable(self):
        return True

    def seekable(self):
        return True

    def tell(self):
        return self.pos

    def seek(self, off, whence=0):
        self.pos = {0: off, 1: self.pos + off, 2: self.size + off}[whence]
        return self.pos

    def _fetch(self, start, end):
        req = urllib.request.Request(self.url, headers={"Range": f"bytes={start}-{end - 1}"})
        for attempt in range(6):
            try:
                with urllib.request.urlopen(req, timeout=120) as r:
                    if r.status != 206:
                        raise OSError(f"expected 206, got {r.status}")
                    data = r.read()
                break
            except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
                # The host rate-limits (HTTP 429); back off and retry.
                if attempt == 5:
                    raise
                print(f"retry after {e}", file=sys.stderr)
                time.sleep(5 * 2 ** attempt)
        if len(data) != end - start:
            raise OSError(f"short range read: {len(data)} of {end - start}")
        return data

    def read(self, n=-1):
        if n is None or n < 0:
            n = self.size - self.pos
        n = max(0, min(n, self.size - self.pos))
        if n == 0:
            return b""
        lo, hi = self.buf_start, self.buf_start + len(self.buf)
        if not (lo <= self.pos and self.pos + n <= hi):
            end = min(self.size, self.pos + max(n, self.BLOCK))
            self.buf_start, self.buf = self.pos, self._fetch(self.pos, end)
        off = self.pos - self.buf_start
        out = self.buf[off:off + n]
        self.pos += len(out)
        return out

    def readinto(self, b):
        data = self.read(len(b))
        b[:len(data)] = data
        return len(data)


def main(argv):
    if len(argv) >= 3 and argv[1] == "list":
        with zipfile.ZipFile(HttpFile(argv[2])) as z:
            for i in z.infolist():
                print(i.file_size, i.filename)
    elif len(argv) >= 5 and argv[1] == "get":
        url, out = argv[2], argv[3]
        os.makedirs(out, exist_ok=True)
        with zipfile.ZipFile(HttpFile(url)) as z:
            # In zip order, so neighbouring members share a read-ahead block.
            names = sorted(argv[4:], key=lambda n: z.getinfo(n).header_offset)
            for name in names:
                dst = os.path.join(out, os.path.basename(name))
                tmp = dst + ".part"
                with z.open(name) as src, open(tmp, "wb") as f:  # ZipExtFile checks the CRC at EOF
                    f.write(src.read())
                os.replace(tmp, dst)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv)
