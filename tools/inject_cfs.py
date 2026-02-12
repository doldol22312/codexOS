#!/usr/bin/env python3
"""Inject or update a host file into a codexOS CFS1 data image."""

from __future__ import annotations

import argparse
import pathlib
import struct
import sys
from dataclasses import dataclass

SECTOR_SIZE = 512
FS_MAGIC = b"CFS1"
FS_VERSION = 1
SUPERBLOCK_LBA = 0
DIRECTORY_START_LBA = 1
DEFAULT_DIRECTORY_SECTORS = 16
ENTRY_SIZE = 64
ENTRIES_PER_SECTOR = SECTOR_SIZE // ENTRY_SIZE
MAX_FILENAME_LEN = 48


class InjectError(Exception):
    pass


@dataclass
class FsState:
    directory_sectors: int
    total_sectors: int
    next_free_lba: int
    file_count: int


@dataclass
class Entry:
    start_lba: int
    size_bytes: int
    allocated_sectors: int


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Inject or replace a file in codexOS CFS1 image"
    )
    parser.add_argument("--image", required=True, help="Path to data.img")
    parser.add_argument("--host", required=True, help="Host file to inject")
    parser.add_argument(
        "--name",
        help="Name inside CFS1 (default: basename of --host)",
    )
    parser.add_argument(
        "--format-if-needed",
        action="store_true",
        help="Initialize CFS1 metadata if the image is unformatted",
    )
    return parser.parse_args()


def read_sector(fd, lba: int) -> bytes:
    fd.seek(lba * SECTOR_SIZE)
    data = fd.read(SECTOR_SIZE)
    if len(data) != SECTOR_SIZE:
        raise InjectError(f"failed to read sector {lba}")
    return data


def write_sector(fd, lba: int, data: bytes) -> None:
    if len(data) != SECTOR_SIZE:
        raise InjectError("internal error: invalid sector size")
    fd.seek(lba * SECTOR_SIZE)
    written = fd.write(data)
    if written != SECTOR_SIZE:
        raise InjectError(f"failed to write sector {lba}")


def decode_superblock(raw: bytes) -> FsState:
    if raw[:4] != FS_MAGIC:
        raise InjectError("invalid CFS1 superblock magic; run fsformat first")

    version = struct.unpack_from("<H", raw, 4)[0]
    if version != FS_VERSION:
        raise InjectError(f"unsupported CFS1 version {version}")

    directory_sectors = struct.unpack_from("<H", raw, 6)[0]
    total_sectors = struct.unpack_from("<I", raw, 8)[0]
    next_free_lba = struct.unpack_from("<I", raw, 12)[0]
    file_count = struct.unpack_from("<I", raw, 16)[0]

    data_start = DIRECTORY_START_LBA + directory_sectors
    if directory_sectors == 0:
        raise InjectError("invalid CFS1: directory size is zero")
    if total_sectors <= data_start:
        raise InjectError("invalid CFS1: total sectors smaller than metadata")
    if next_free_lba < data_start or next_free_lba > total_sectors:
        raise InjectError("invalid CFS1: next_free_lba out of range")

    return FsState(
        directory_sectors=directory_sectors,
        total_sectors=total_sectors,
        next_free_lba=next_free_lba,
        file_count=file_count,
    )


def encode_superblock(state: FsState) -> bytes:
    raw = bytearray(SECTOR_SIZE)
    raw[0:4] = FS_MAGIC
    struct.pack_into("<H", raw, 4, FS_VERSION)
    struct.pack_into("<H", raw, 6, state.directory_sectors)
    struct.pack_into("<I", raw, 8, state.total_sectors)
    struct.pack_into("<I", raw, 12, state.next_free_lba)
    struct.pack_into("<I", raw, 16, state.file_count)
    return bytes(raw)


def format_filesystem(fd) -> FsState:
    fd.seek(0, 2)
    image_bytes = fd.tell()
    if image_bytes % SECTOR_SIZE != 0:
        raise InjectError("data image size is not a multiple of 512 bytes")

    total_sectors = image_bytes // SECTOR_SIZE
    if total_sectors <= DIRECTORY_START_LBA + 1:
        raise InjectError("data image is too small for CFS1 metadata")

    max_directory = total_sectors - DIRECTORY_START_LBA - 1
    directory_sectors = min(DEFAULT_DIRECTORY_SECTORS, max_directory)
    if directory_sectors <= 0:
        directory_sectors = 1

    data_start = DIRECTORY_START_LBA + directory_sectors
    state = FsState(
        directory_sectors=directory_sectors,
        total_sectors=total_sectors,
        next_free_lba=data_start,
        file_count=0,
    )

    write_sector(fd, SUPERBLOCK_LBA, encode_superblock(state))
    zero_sector = bytes(SECTOR_SIZE)
    for index in range(directory_sectors):
        write_sector(fd, DIRECTORY_START_LBA + index, zero_sector)

    return state


def validate_name(name: str) -> bytes:
    encoded = name.encode("ascii", errors="strict")
    if not encoded:
        raise InjectError("CFS filename cannot be empty")
    if len(encoded) > MAX_FILENAME_LEN:
        raise InjectError(
            f"CFS filename too long ({len(encoded)} > {MAX_FILENAME_LEN})"
        )
    for byte in encoded:
        if not (
            (ord("a") <= byte <= ord("z"))
            or (ord("A") <= byte <= ord("Z"))
            or (ord("0") <= byte <= ord("9"))
            or byte in (ord("."), ord("_"), ord("-"))
        ):
            raise InjectError(
                "CFS filename has invalid characters; use [A-Za-z0-9._-]"
            )
    return encoded


def decode_entry(raw: bytes) -> Entry:
    return Entry(
        start_lba=struct.unpack_from("<I", raw, 4)[0],
        size_bytes=struct.unpack_from("<I", raw, 8)[0],
        allocated_sectors=struct.unpack_from("<I", raw, 12)[0],
    )


def encode_entry(name_bytes: bytes, entry: Entry) -> bytes:
    raw = bytearray(ENTRY_SIZE)
    raw[0] = 0x01
    raw[1] = len(name_bytes)
    struct.pack_into("<I", raw, 4, entry.start_lba)
    struct.pack_into("<I", raw, 8, entry.size_bytes)
    struct.pack_into("<I", raw, 12, entry.allocated_sectors)
    raw[16 : 16 + len(name_bytes)] = name_bytes
    return bytes(raw)


def scan_directory(fd, state: FsState, name_bytes: bytes):
    existing = None
    free = None

    for sector_idx in range(state.directory_sectors):
        lba = DIRECTORY_START_LBA + sector_idx
        sector = bytearray(read_sector(fd, lba))

        for slot_idx in range(ENTRIES_PER_SECTOR):
            offset = slot_idx * ENTRY_SIZE
            entry_raw = sector[offset : offset + ENTRY_SIZE]
            used = (entry_raw[0] & 0x01) != 0

            if used:
                name_len = min(entry_raw[1], MAX_FILENAME_LEN)
                existing_name = bytes(entry_raw[16 : 16 + name_len])
                if existing_name == name_bytes:
                    existing = (lba, slot_idx, sector, decode_entry(entry_raw))
                    return existing, free
            elif free is None:
                free = (lba, slot_idx, sector)

    return existing, free


def write_extent(fd, start_lba: int, payload: bytes, sectors: int) -> None:
    for idx in range(sectors):
        begin = idx * SECTOR_SIZE
        chunk = payload[begin : begin + SECTOR_SIZE]
        padded = chunk + b"\0" * (SECTOR_SIZE - len(chunk))
        write_sector(fd, start_lba + idx, padded)


def main() -> int:
    args = parse_args()

    image_path = pathlib.Path(args.image)
    host_path = pathlib.Path(args.host)
    cfs_name = args.name or host_path.name

    if not image_path.is_file():
        raise InjectError(f"image not found: {image_path}")
    if not host_path.is_file():
        raise InjectError(f"host file not found: {host_path}")

    name_bytes = validate_name(cfs_name)
    payload = host_path.read_bytes()
    if len(payload) == 0:
        raise InjectError("host file is empty")

    required_sectors = (len(payload) + SECTOR_SIZE - 1) // SECTOR_SIZE

    with image_path.open("r+b") as fd:
        try:
            state = decode_superblock(read_sector(fd, SUPERBLOCK_LBA))
        except InjectError:
            if not args.format_if_needed:
                raise
            state = format_filesystem(fd)
            print(
                f"Initialized CFS1 filesystem in {image_path} "
                f"({state.total_sectors} sectors)"
            )

        existing, free = scan_directory(fd, state, name_bytes)

        if existing is not None:
            lba, slot, sector, old_entry = existing
            if old_entry.allocated_sectors >= required_sectors and old_entry.start_lba != 0:
                start_lba = old_entry.start_lba
                allocated = old_entry.allocated_sectors
            else:
                start_lba = state.next_free_lba
                allocated = required_sectors
                end_lba = start_lba + allocated
                if end_lba > state.total_sectors:
                    raise InjectError("not enough free sectors in data image")
                state.next_free_lba = end_lba

            write_extent(fd, start_lba, payload, required_sectors)
            new_entry = Entry(
                start_lba=start_lba,
                size_bytes=len(payload),
                allocated_sectors=allocated,
            )
            offset = slot * ENTRY_SIZE
            sector[offset : offset + ENTRY_SIZE] = encode_entry(name_bytes, new_entry)
            write_sector(fd, lba, bytes(sector))
            write_sector(fd, SUPERBLOCK_LBA, encode_superblock(state))

            print(
                f"Updated {cfs_name}: {len(payload)} bytes, {required_sectors} sectors"
                f" (start LBA {start_lba})"
            )
            return 0

        if free is None:
            raise InjectError("filesystem directory is full")

        start_lba = state.next_free_lba
        end_lba = start_lba + required_sectors
        if end_lba > state.total_sectors:
            raise InjectError("not enough free sectors in data image")

        write_extent(fd, start_lba, payload, required_sectors)

        lba, slot, sector = free
        entry = Entry(
            start_lba=start_lba,
            size_bytes=len(payload),
            allocated_sectors=required_sectors,
        )
        offset = slot * ENTRY_SIZE
        sector[offset : offset + ENTRY_SIZE] = encode_entry(name_bytes, entry)
        write_sector(fd, lba, bytes(sector))

        state.next_free_lba = end_lba
        state.file_count += 1
        write_sector(fd, SUPERBLOCK_LBA, encode_superblock(state))

        print(
            f"Injected {cfs_name}: {len(payload)} bytes, {required_sectors} sectors"
            f" (start LBA {start_lba})"
        )

    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except InjectError as error:
        print(f"inject_cfs.py: error: {error}", file=sys.stderr)
        sys.exit(1)
