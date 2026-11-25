"""LCAN/1 canonical encoding for CAD-0 contract fixtures.

The schema/adapter supplies the typed tree and selects normalization rules.
"""
import hashlib
import json
import math
import re
import struct
from fractions import Fraction


def quote(value):
    value.encode("utf-8")  # Reject Python strings containing unpaired surrogates.
    quoted = json.dumps(value, ensure_ascii=False)
    for escaped, code in (("b", "08"), ("f", "0c"), ("n", "0a"), ("r", "0d"), ("t", "09")):
        # Short control escapes can follow a run of escaped literal backslashes.
        quoted = re.sub(r'(?<!\\)((?:\\\\)*)\\' + escaped,
                        lambda match: match[1] + r"\u00" + code, quoted)
    return quoted


def integer(value, signed=False):
    pattern = r"(?:0|-?[1-9][0-9]*)" if signed else r"(?:0|[1-9][0-9]*)"
    if not isinstance(value, str) or not re.fullmatch(pattern, value):
        raise ValueError("Invalid integer encoding")
    number = int(value)
    minimum, maximum = (-(2**63), 2**63 - 1) if signed else (0, 2**64 - 1)
    if not minimum <= number <= maximum:
        raise ValueError("Integer out of range")
    return number


def f64(value):
    if not isinstance(value, str) or not re.fullmatch(
        r"-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?", value
    ):
        raise ValueError("Invalid finite decimal")
    number = float(value)
    if not math.isfinite(number):
        raise ValueError("Expected finite binary64")
    return struct.pack(">d", 0.0 if number == 0.0 else number).hex()


def dependency_order(nodes):
    remaining = {node["key"]: node for node in nodes}
    if len(remaining) != len(nodes):
        raise ValueError("Duplicate recipe key")
    ordered = []
    emitted = set()
    while remaining:
        ready = [key for key, node in remaining.items() if set(node["dependencies"]) <= emitted]
        if not ready:
            raise ValueError("Cyclic DAG or missing dependency")
        key = min(ready, key=lambda item: item.encode("utf-8"))
        ordered.append(remaining.pop(key))
        emitted.add(key)
    return ordered


def normalize(value):
    if isinstance(value, list):
        return [normalize(item) for item in value]
    if not isinstance(value, dict):
        return value
    if len(value) == 1:
        tag, payload = next(iter(value.items()))
        if tag == "$f64":
            return {tag: f64(payload)}
        if tag in ("$u64", "$i64"):
            return {tag: str(integer(payload, signed=tag == "$i64"))}
        if tag == "$rational":
            numerator = integer(payload["numerator"], signed=True)
            denominator = integer(payload["denominator"])
            if denominator == 0:
                raise ValueError("Rational denominator must be positive")
            value = Fraction(numerator, denominator)
            return {tag: {"numerator": str(value.numerator), "denominator": str(value.denominator)}}
        if tag == "$fixture_plan_ref":
            return {key: payload[key] for key in ("logical_key", "producer_output")}
        if tag == "$fixture_keyed_set":
            seen = {entry["key"] for entry in payload}
            if len(seen) != len(payload):
                raise ValueError("Duplicate set key")
            return normalize(sorted(payload, key=lambda item: item["key"].encode("utf-8")))
        if tag == "$fixture_recipe":
            return normalize(dependency_order(payload))
    if any(key.startswith("$") for key in value):
        raise ValueError("Unknown LCAN tag")
    return {key: normalize(item) for key, item in value.items()}


def serialize(value):
    if isinstance(value, str):
        return quote(value)
    if value is None or isinstance(value, bool):
        return json.dumps(value)
    if isinstance(value, list):
        return "[" + ",".join(map(serialize, value)) + "]"
    if isinstance(value, dict):
        keys = sorted(value, key=lambda key: key.encode("utf-8"))
        return "{" + ",".join(quote(key) + ":" + serialize(value[key]) for key in keys) + "}"
    raise ValueError("Numbers must be tagged in the typed tree")


def encode(value):
    return serialize(normalize(value)).encode("utf-8")


def digest(domain, payload):
    if not domain or "\0" in domain:
        raise ValueError("Invalid LCAN domain")
    header = domain.encode("utf-8") + b"\0LCAN/1\0" + len(payload).to_bytes(8, "big")
    return "sha256:" + hashlib.sha256(header + payload).hexdigest()
