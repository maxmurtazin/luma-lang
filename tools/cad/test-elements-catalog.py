"""Exercise the Luma catalog reader across installation of a new Elements package."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

import lcan


ROOT = Path(__file__).resolve().parents[2]


def call(executable, *args, code=0):
    completed = subprocess.run([executable, *args], capture_output=True, check=False)
    assert completed.returncode == code, (args, completed.stdout, completed.stderr)
    return json.loads(completed.stdout)


def write(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False), encoding="utf-8")
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reader", type=Path)
    parser.add_argument("elements_cli", type=Path)
    parser.add_argument("checkout", type=Path)
    args = parser.parse_args()
    elements = args.elements_cli.resolve()
    reader = args.reader.resolve()
    fixtures = args.checkout / "fixtures/local"
    commands = ROOT / "examples/cad"
    find = commands / "elements-find-command.json"
    resolve = commands / "elements-resolve-command.json"
    with tempfile.TemporaryDirectory() as directory:
        scratch = Path(directory)
        database = scratch / "catalog.sqlite"
        original = scratch / "original.lirelp"
        call(elements, "catalog-init", database)
        call(elements, "package-build", fixtures / "plate-package.json", original)
        call(elements, "catalog-install", database, original)

        captured = call(reader, database, find)["result"]
        outputs = captured["outputs"]
        assert outputs["matches"]["value"]["type_name"] == "ElementSearchResult"
        pin = outputs["pin"]["value"]["payload"]
        exact = write(scratch / "pin.json", pin)
        original_definition = outputs["part"]["value"]["payload"]
        assert pin["source"]["package_version"] == "1.0.0"
        assert original_definition["definition"]["body"]["variants"][0]["values"][0]["value"]["value"] == 0.04
        assert original_definition["required_package_pins"] == [pin["source"]]
        assert original_definition == call(reader, database, resolve, exact)["result"]["outputs"]["part"]["value"]["payload"]

        project = json.loads((fixtures / "plate-project.json").read_text(encoding="utf-8"))
        project["instances"][1]["source_configuration"]["variant_id"] = (
            original_definition["definition"]["body"]["variants"][0]["variant_id"]
        )
        archive = scratch / "archived.lirela"
        call(elements, "project-save", database, write(scratch / "project.json", project), archive)
        before = call(elements, "project-open", archive)
        bom = call(elements, "project-bom", archive)

        newer = json.loads((fixtures / "plate-package.json").read_text(encoding="utf-8"))
        newer["package_version"] = "1.1.0"
        new_definition = newer["definitions"][0]
        body = new_definition["body"]
        body["revision_id"] = "20000000-0000-4000-8000-000000000002"
        body["variants"][0]["values"][0]["value"]["value"] = 0.05
        new_definition["revision_digest"] = call(
            elements, "definition-digest", write(scratch / "body.json", body)
        )["revision_digest"]
        replacement = scratch / "replacement.lirelp"
        call(elements, "package-build", write(scratch / "replacement.json", newer), replacement)
        receipt = call(elements, "catalog-install", database, replacement)
        new_pin = {
            "source": {key: receipt["package"][key] for key in pin["source"]},
            "definition_id": body["definition_id"],
            "revision_id": body["revision_id"],
            "expected_revision_digest": new_definition["revision_digest"],
        }
        resolved = call(reader, database, resolve, write(scratch / "new-pin.json", new_pin))["result"]["outputs"]
        assert resolved["part"]["value"]["payload"]["definition"]["body"] == body
        assert call(reader, database, resolve, exact)["result"]["outputs"]["part"]["value"]["payload"] == original_definition
        assert call(elements, "project-open", archive) == before
        assert call(elements, "project-bom", archive) == bom
        entry = "write.luma"
        files = {entry: "use elements\nelements.place()\n"}
        target = {"bundle_version": {"$u64": "1"}, "entry": entry, "files": files}
        write_command = write(scratch / "write.json", {
            **target,
            "bundle_version": 1,
            "source_digest": lcan.digest("luma.cad.source", lcan.encode(target)),
        })
        catalog_before = database.read_bytes()
        denied = call(reader, database, write_command, code=2)
        assert denied["error"]["diagnostics"][0]["code"] == "UnsupportedCapability"
        assert database.read_bytes() == catalog_before

        tampered = {**pin, "expected_revision_digest": "sha256:" + "0" * 64}
        denied = call(reader, database, resolve, write(scratch / "bad.json", tampered), code=2)
        assert denied["error"]["code"] == "REVISION_DIGEST_MISMATCH"
        duplicate = scratch / "duplicate.json"
        duplicate.write_text('{"revision_id":null,' + json.dumps(pin)[1:], encoding="utf-8")
        assert "result" not in call(reader, database, resolve, duplicate, code=2)
        empty = scratch / "empty.sqlite"
        call(elements, "catalog-init", empty)
        assert "result" not in call(reader, empty, resolve, exact, code=2)
    print("PASS Elements catalog: exact package and definition pins, variants and historical project BOM")


if __name__ == "__main__":
    main()
