"""Mutation-row anchors: Rust token spans, or exact substrings for other files."""
from functools import lru_cache
from bisect import bisect_left, bisect_right
import re
from collections import namedtuple

Token = namedtuple("Token", "value start end")


class AnchorError(ValueError):
    pass


def _quoted_end(text, quote):
    i = quote + 1
    while i < len(text):
        if text[i] == "\\":
            i += 2
        elif text[i] == '"':
            return i + 1
        else:
            i += 1
    return len(text)


def _raw_end(text, start):
    if text.startswith(("br", "cr"), start):
        i = start + 2
    elif text[start] == "r":
        i = start + 1
    else:
        return None
    hashes = 0
    while i < len(text) and text[i] == "#":
        hashes += 1
        i += 1
    if i == len(text) or text[i] != '"':
        return None
    closing = '"' + "#" * hashes
    end = text.find(closing, i + 1)
    return len(text) if end < 0 else end + len(closing)


def _char_end(text, quote):
    i = quote + 1
    if i == len(text):
        return None
    if text[i] == "\\":
        i += 1
        if i == len(text):
            return None
        if text[i] == "u" and i + 1 < len(text) and text[i + 1] == "{":
            end = text.find("}", i + 2)
            i = len(text) if end < 0 else end + 1
        elif text[i] == "x":
            i += 3
        else:
            i += 1
    else:
        i += 1
    return i + 1 if i < len(text) and text[i] == "'" else None


def _identifier_start(char):
    return char == "_" or char.isidentifier()


def _identifier_continue(char):
    return char == "_" or ("a" + char).isidentifier()


def _identifier_end(text, start):
    i = start + 1
    while i < len(text) and _identifier_continue(text[i]):
        i += 1
    return i


def _digits_end(text, start, allowed):
    i, found = start, False
    while i < len(text) and (text[i] == "_" or text[i] in allowed):
        found |= text[i] in allowed
        i += 1
    return i, found


def _number_end(text, start, after_dot):
    size = len(text)
    if text.startswith(("0x", "0o", "0b"), start):
        allowed = {"x": "0123456789abcdefABCDEF", "o": "01234567", "b": "01"}[text[start + 1]]
        i, found = _digits_end(text, start + 2, allowed)
        if not found:
            raise AnchorError("base-prefixed number has no digits")
        if i < size and text[i].isascii() and text[i].isdigit() and text[i] not in allowed:
            raise AnchorError("base-prefixed number contains an invalid digit")
    else:
        i, _ = _digits_end(text, start, "0123456789")
        if not after_dot and i < size and text[i] == "." and not text.startswith("..", i) and not (i + 1 < size and _identifier_start(text[i + 1])):
            i += 1
            i, _ = _digits_end(text, i, "0123456789")
        if i < size and text[i] in "eE":
            exponent = i + 1
            if exponent < size and text[exponent] in "+-":
                exponent += 1
                if exponent == size or not (text[exponent].isascii() and text[exponent].isdigit()):
                    raise AnchorError("number exponent has no digits")
                i, _ = _digits_end(text, exponent, "0123456789")
            elif exponent < size and text[exponent].isascii() and text[exponent].isdigit():
                i, _ = _digits_end(text, exponent, "0123456789")

    if i < size and _identifier_start(text[i]):
        i = _identifier_end(text, i)
    return i


def _tokens(text):
    out, i, size = [], 0, len(text)
    while i < size:
        start, char = i, text[i]
        if char.isspace():
            i += 1
            continue
        if text.startswith("//", i):
            end = i + 2
            while end < size and text[end] not in "\r\n":
                end += 1
            value = re.sub(r"\s+", " ", text[start:end])
            out.append(Token(value, start, end))
            i = end
            continue
        if text.startswith("/*", i):
            end, depth = i + 2, 1
            while end < size and depth:
                if text.startswith("/*", end):
                    depth += 1
                    end += 2
                elif text.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            value = re.sub(r"\s+", " ", text[start:end])
            out.append(Token(value, start, end))
            i = end
            continue

        raw_end = _raw_end(text, i)
        if raw_end is not None:
            out.append(Token(text[start:raw_end], start, raw_end))
            i = raw_end
            continue
        if char in "bc" and i + 1 < size and text[i + 1] == '"':
            end = _quoted_end(text, i + 1)
            out.append(Token(text[start:end], start, end))
            i = end
            continue
        if char == '"':
            end = _quoted_end(text, i)
            out.append(Token(text[start:end], start, end))
            i = end
            continue
        if char == "b" and i + 1 < size and text[i + 1] == "'":
            end = _char_end(text, i + 1)
            if end is not None:
                out.append(Token(text[start:end], start, end))
                i = end
                continue
        if char == "'":
            end = _char_end(text, i)
            if end is not None:
                out.append(Token(text[start:end], start, end))
                i = end
                continue
        if text.startswith("r#", i):
            if i + 2 == size or not _identifier_start(text[i + 2]):
                raise AnchorError("invalid raw identifier")
            i = _identifier_end(text, i + 2)
            out.append(Token(text[start:i], start, i))
            continue
        if char.isascii() and char.isdigit():
            i = _number_end(text, i, bool(out and out[-1].value == "."))
            out.append(Token(text[start:i], start, i))
            continue
        if _identifier_start(char):
            i = _identifier_end(text, i)
            out.append(Token(text[start:i], start, i))
            continue
        if not char.isascii() and char.isalnum():
            raise AnchorError("unsupported non-ASCII digit outside an identifier")
        i += 1
        out.append(Token(char, start, i))
    return out


@lru_cache(maxsize=128)
def _source_index(text):
    tokens, positions = _tokens(text), {}
    for i, token in enumerate(tokens):
        positions.setdefault(token.value, []).append(i)
    return tokens, positions


def _token_matches(tokens, positions, pattern, start=0, end=None):
    end = len(tokens) if end is None else end
    width = len(pattern)
    if not width:
        return []
    wanted = [token.value for token in pattern]
    candidates = positions.get(wanted[0], ())
    first = bisect_left(candidates, start)
    last = bisect_left(candidates, end - width + 1)
    return [i for i in candidates[first:last]
            if all(tokens[i + j].value == wanted[j] for j in range(1, width))]


def _scope_range(tokens, positions, scope):
    pattern = _tokens(scope)
    scope_hits = _token_matches(tokens, positions, pattern)
    if len(scope_hits) != 1:
        raise AnchorError(f"scope matches {len(scope_hits)} times")
    scope_end = scope_hits[0] + len(pattern) - 1
    braces = positions.get("{", ())
    if tokens[scope_end].value == "{":
        open_at = scope_end
    else:
        next_brace = bisect_right(braces, scope_end)
        if next_brace == len(braces):
            raise AnchorError("scope has no opening { block")
        open_at = braces[next_brace]
    depth = 1
    for i in range(open_at + 1, len(tokens)):
        if tokens[i].value == "{":
            depth += 1
        elif tokens[i].value == "}":
            depth -= 1
            if depth == 0:
                return open_at, i + 1
    raise AnchorError("scope block has no matching }")


def find(text, old, scope=None, path=""):
    """Return exact spans, or Rust token spans ignoring inter-token whitespace."""
    if not path.endswith(".rs"):
        if scope is not None:
            raise AnchorError("scope is supported only for .rs files")
        if not old:
            return [(i, i) for i in range(len(text) + 1)]
        spans, start = [], 0
        while True:
            at = text.find(old, start)
            if at < 0:
                return spans
            spans.append((at, at + len(old)))
            start = at + len(old)

    tokens, positions = _source_index(text)
    pattern = _tokens(old)
    if not pattern:
        return []
    start, end = _scope_range(tokens, positions, scope) if scope is not None else (0, len(tokens))
    return [(tokens[i].start, tokens[i + len(pattern) - 1].end)
            for i in _token_matches(tokens, positions, pattern, start, end)]


def _replace(text, span, replacement):
    return text[:span[0]] + replacement + text[span[1]:]


def _one(text, old, scope, path, label):
    spans = find(text, old, scope, path)
    if len(spans) != 1:
        after = " after old is replaced" if label == "old2" else ""
        raise AnchorError(f"{label} occurs {len(spans)} times{after}")
    return spans[0]


def apply(text, row):
    """Apply a row's old/new edit, then its optional old2/new2 edit."""
    path, scope = row["file"], row.get("scope")
    first = _one(text, row["old"], scope, path, "old")
    mutated = _replace(text, first, row["new"])
    if "old2" in row:
        second = _one(mutated, row["old2"], scope, path, "old2")
        mutated = _replace(mutated, second, row["new2"])
    return mutated
