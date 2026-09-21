# SPDX-License-Identifier: MIT
"""Synthetic linear DNG, with no real camera or GPS data."""

import struct


def write_raw_photo(path):
    def short(tag, value):
        return tag, 3, 1, struct.pack("<H", value)

    def long(tag, value):
        return tag, 4, 1, struct.pack("<I", value)

    def text(tag, value):
        value = value.encode() + b"\0"
        return tag, 2, len(value), value

    def rational(tag, values):
        return tag, 5, len(values), b"".join(struct.pack("<II", *value) for value in values)

    data = bytearray(b"II\x2a\0\0\0\0\0")

    def directory(tags):
        tags = sorted(tags)
        offset = len(data)
        data.extend(struct.pack("<H", len(tags)))
        data.extend(bytes(len(tags) * 12 + 4))
        for index, (tag, kind, count, value) in enumerate(tags):
            position = offset + 2 + index * 12
            struct.pack_into("<HHI", data, position, tag, kind, count)
            if len(value) <= 4:
                data[position + 8:position + 8 + len(value)] = value
            else:
                struct.pack_into("<I", data, position + 8, len(data))
                data.extend(value)
                if len(data) % 2:
                    data.append(0)
        return offset

    exif = directory([
        rational(33434, [(1, 250)]), short(34855, 400),
        rational(37386, [(50, 1)]), text(42036, "Synthetic 50mm lens"),
        long(40962, 600), long(40963, 400),
    ])
    gps = directory([
        text(1, "S"), rational(2, [(12, 1), (30, 1), (0, 1)]),
        text(3, "W"), rational(4, [(45, 1), (15, 1), (0, 1)]),
    ])
    width, height = 600, 400
    tags = [
        long(256, width), long(257, height), (258, 3, 3, struct.pack("<HHH", 8, 8, 8)),
        short(259, 1), short(262, 34892), text(271, "Strata"), text(272, "Test Camera"),
        long(273, 0), short(274, 1), short(277, 3), long(278, height),
        long(279, width * height * 3), long(34665, exif), long(34853, gps),
        (50706, 1, 4, bytes([1, 4, 0, 0])), text(50708, "Strata Test Camera"),
    ]
    root = directory(tags)
    def pixels(offset, fields, width, height):
        strip_index = [tag[0] for tag in sorted(fields)].index(273)
        struct.pack_into("<I", data, offset + 2 + strip_index * 12 + 8, len(data))
        for y in range(height):
            for x in range(width):
                data.extend((x * 255 // width, y * 255 // height, 100))

    pixels(root, tags, width, height)
    # The ordinary RGB preview is intentionally smaller than the RAW SubIFD.
    # Dimensions must come from the original image, not this preview directory.
    preview_width, preview_height = 300, 200
    preview_tags = [
        long(254, 1), long(256, preview_width), long(257, preview_height),
        (258, 3, 3, struct.pack("<HHH", 8, 8, 8)), short(259, 1), short(262, 2),
        text(271, "Strata"), text(272, "Test Camera"), long(273, 0), short(274, 1),
        short(277, 3), long(278, preview_height), long(279, preview_width * preview_height * 3),
        long(330, root), long(34665, exif), long(34853, gps),
        (50706, 1, 4, bytes([1, 4, 0, 0])), text(50708, "Strata Test Camera"),
    ]
    preview = directory(preview_tags)
    struct.pack_into("<I", data, 4, preview)
    pixels(preview, preview_tags, preview_width, preview_height)
    path.write_bytes(data)
