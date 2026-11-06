"""固定离线 AI 资产的构建与安装器共用验证；不下载或执行资产。"""

from __future__ import annotations

import hashlib
import json
import re
import stat
from pathlib import Path
from typing import Mapping

FILES = frozenset({
    "model_int8.onnx", "onnxruntime.dll", "vocab.txt", "LICENSE-model.txt",
    "LICENSE-onnxruntime.txt", "ThirdPartyNotices-onnxruntime.txt",
    "README-model.md", "README-conversion.md",
})
BUNDLE_FILES = FILES | {"manifest.json"}
MODEL_SIZE = 103126077
MODEL_SHA256 = "11fd06011b1381bcb2f5ac1cd9c208d502ce652876c79c2903bcd786e0bfb97b"
MODEL = {
    "repository": "Xenova/macbert4csc-base-chinese",
    "revision": "7ebfe81cf502576e93c844b95354840b2ecb5c28",
    "license": "Apache-2.0",
    "upstream_repository": "shibing624/macbert4csc-base-chinese",
    "upstream_revision": "615e6e09ef9a69ec487bc7c641ec3a311e2c11b9",
}
ARCHIVE_URL = (
    "https://github.com/microsoft/onnxruntime/releases/download/"
    "v1.20.0/onnxruntime-win-x64-1.20.0.zip"
)
MAX_PAYLOAD = 256 * 1024 * 1024
MAX_MANIFEST = 64 * 1024


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _plain(path: Path, directory: bool = False) -> None:
    info = path.lstat()
    if (info.st_file_attributes if hasattr(info, "st_file_attributes") else 0) & 0x400:
        raise ValueError(f"AI 路径不能是 reparse point：{path}")
    if not (stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode)):
        raise ValueError(f"AI 路径不是普通{'目录' if directory else '文件'}：{path}")


def _parents(path: Path) -> None:
    for parent in reversed(path.absolute().parents):
        _plain(parent, directory=True)


def _read(path: Path, limit: int) -> bytes:
    _parents(path)
    _plain(path)
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    if not data or len(data) > limit:
        raise ValueError(f"AI 文件为空或超过大小边界：{path.name}")
    return data


def _unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"AI manifest 包含重复字段：{key}")
        result[key] = value
    return result


def source_manifest(repo: Path) -> tuple[bytes, dict]:
    data = _read(repo / "data/ai/manifest.json", MAX_MANIFEST)
    record = json.loads(data, object_pairs_hook=_unique)
    if (not isinstance(record, dict)
            or set(record) != {"schema_version", "model", "runtime", "files"}
            or type(record["schema_version"]) is not int
            or record["schema_version"] != 1 or record["model"] != MODEL):
        raise ValueError("AI 源 manifest 模型身份或 schema 不匹配")
    runtime = record["runtime"]
    if (not isinstance(runtime, dict)
            or set(runtime) != {"version", "license", "archive_url", "archive_sha256"}
            or runtime["version"] != "1.20.0" or runtime["license"] != "MIT"
            or runtime["archive_url"] != ARCHIVE_URL
            or not isinstance(runtime["archive_sha256"], str)
            or not re.fullmatch("[0-9a-f]{64}", runtime["archive_sha256"])):
        raise ValueError("AI 源 manifest runtime 来源或许可不匹配")
    rows = record["files"]
    if not isinstance(rows, list) or len(rows) != len(FILES):
        raise ValueError("AI 源 manifest 必须包含固定八文件记录")
    seen = set()
    total = len(data)
    for row in rows:
        if (not isinstance(row, dict) or set(row) != {"path", "size", "sha256"}
                or not isinstance(row["path"], str) or row["path"] not in FILES
                or row["path"] in seen or type(row["size"]) is not int
                or not 0 < row["size"] <= MAX_PAYLOAD
                or not isinstance(row["sha256"], str)
                or not re.fullmatch("[0-9a-f]{64}", row["sha256"])):
            raise ValueError("AI 源 manifest 文件路径、大小或摘要无效")
        seen.add(row["path"])
        total += row["size"]
        if row["path"] == "model_int8.onnx" and (
            row["size"] != MODEL_SIZE or row["sha256"] != MODEL_SHA256
        ):
            raise ValueError("AI 模型固定大小或摘要不匹配")
    if seen != FILES or total > MAX_PAYLOAD:
        raise ValueError("AI 文件集合或总大小超出边界")
    return data, record


def verify_payload(payload: Mapping[str, bytes], repo: Path) -> dict:
    """只信任仓库固定源记录，包内记录不自行建立信任。"""
    source, record = source_manifest(repo)
    if set(payload) != BUNDLE_FILES or payload["manifest.json"] != source:
        raise ValueError("AI 文件集合或源 manifest 逐字校验失败")
    for row in record["files"]:
        data = payload[row["path"]]
        if len(data) != row["size"] or sha256(data) != row["sha256"]:
            raise ValueError(f"AI 文件大小或 SHA-256 不匹配：{row['path']}")
    return {
        "available": True,
        "source_manifest_sha256": sha256(source),
        "source": record,
    }


def load_bundle(directory: Path, repo: Path) -> tuple[dict[str, bytes], dict]:
    # 不先 resolve：必须看见用户输入路径本身及父路径上的链接/reparse。
    directory = directory.expanduser().absolute()
    _parents(directory)
    _plain(directory, directory=True)
    entries = list(directory.iterdir())
    if {path.name for path in entries} != BUNDLE_FILES:
        raise ValueError("AI 目录必须恰好包含固定九文件，不接受额外 DLL 或目录")
    source, record = source_manifest(repo)
    limits = {row["path"]: row["size"] for row in record["files"]}
    limits["manifest.json"] = len(source)
    payload = {path.name: _read(path, limits[path.name]) for path in entries}
    ai = verify_payload(payload, repo)
    return {"ai/" + name: data for name, data in payload.items()}, ai
