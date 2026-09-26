"""Extract a release dependency archive without Windows tar.exe."""

import pathlib
import shutil
import sys
import tarfile


def extract(archive: pathlib.Path, destination: pathlib.Path, strip: int) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, "r:*") as source:
        for member in source:
            name = member.name
            if name.startswith("/") or "\\" in name:
                raise ValueError(f"Unsafe archive path: {name}")
            parts = [part for part in name.split("/") if part not in ("", ".")]
            if ".." in parts or (parts and ":" in parts[0]):
                raise ValueError(f"Unsafe archive path: {name}")
            if len(parts) <= strip:
                continue
            target = destination.joinpath(*parts[strip:])
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif member.isfile() or member.issym() or member.islnk():
                contents = source.extractfile(member)
                if contents is None:
                    raise ValueError(f"Cannot read archive entry: {name}")
                target.parent.mkdir(parents=True, exist_ok=True)
                with contents, target.open("wb") as output:
                    shutil.copyfileobj(contents, output)
            else:
                raise ValueError(f"Unsupported archive entry: {name}")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        raise SystemExit("Usage: extract-windows-dependency.py ARCHIVE DESTINATION STRIP")
    extract(pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), int(sys.argv[3]))
