"""Optional GPT brief checks. Parsing helpers are supplied by brief_lint, never imported."""
import pathlib
import re


CAP = re.compile(
    r"\b(?:past|under|below|over|at most|cap of)\s+(?:about\s+)?(\d[\d,]*)\s+lines\b"
    r"|\b(\d[\d,]*)-line cap\b", re.I)
POINTER = re.compile(r"(?<![\w/\\])(?:scratch|state)[/\\](?:[^\s`'\"()\[\],;]|\[[^\s`'\"()\[\],;]*\])+", re.I)


def caps(text):
    values = []
    for match in CAP.finditer(text):
        try:
            values.append(int((match[1] or match[2]).replace(",", "")))
        except ValueError:
            continue
    return values


def check(text, path, options, split_sections, claimed_paths, claim_blocks, claim_paths):
    """Return FAIL triples, or an empty list. Read files only for enabled size/cap rules."""
    for name in ("brief_max_kb", "size_pct", "cap_margin_pct"):
        value = getattr(options, name)
        if value is not None and (type(value) is not int or value < 1):
            raise ValueError(f"{name} must be a positive integer")
    if (options.brief_max_kb is None) != (options.size_pct is None):
        raise ValueError("--brief-max-kb and --size-pct are both needed")
    if options.cap_margin_pct is not None and not options.root:
        raise ValueError("--cap-margin-pct needs --root")
    if isinstance(text, bytes):
        text = text.decode("utf-8", errors="replace")
    # Keep physical line numbers even when a comment spans lines or is unterminated.
    clean = re.sub(r"<!--.*?(?:-->|$)",
                   lambda m: re.sub(r"[^\r\n]", " ", m[0]), text, flags=re.S)
    lines = clean.splitlines()
    sections = split_sections(lines)
    result = []
    if options.gpt_preflight:
        skipped = set()
        for name, start, end in sections:
            if re.search(r"Evidence|Pasted|Board|After merge", name, re.I):
                skipped.update(range(start, end))
        pointers = []
        for i, line in enumerate(lines):
            if i in skipped or not re.search(r"\b(?:see|read|open|consult|follow|per)\b", line, re.I):
                continue
            if re.search(r"pasted|quoted|orchestrator|\(manual\)", line, re.I):
                continue
            for match in POINTER.finditer(line):
                p = match[0].rstrip(".:")
                if "<" not in p and "*" not in p and len(pointers) < 5:
                    pointers.append(f"line {i + 1}: {p}")
        if pointers:
            result.append(("B1", "FAIL", "; ".join(pointers)
                           + "; paste the content (a coder cannot read scratch/ or state/)"))
        if re.search(r"cannot\s+run\s+commands", text, re.I) and not re.search(
                r"you can read every file in your worktree", " ".join(text.split()), re.I):
            result.append(("B3", "FAIL", '"cannot run commands" without '
                           '"you can read every file in your worktree" '
                           '(GPT coders read it as "cannot read files")'))
    if options.brief_max_kb is not None:
        n, pct = options.brief_max_kb, options.size_pct
        try:
            size = pathlib.Path(path).stat().st_size
        except (OSError, ValueError) as error:
            result.append(("B2", "FAIL", f"cannot read brief byte size: {error}"))
        else:
            numerator = n * 1024 * pct
            if size * 100 > numerator:
                limit = numerator // 100
                result.append(("B2", "FAIL", f"{size} bytes, over {pct}% of "
                               f"reviewer_brief_max_kb = {n} ({limit} bytes): "
                               "replace sections in an amendment instead of appending"))
    if options.cap_margin_pct is not None and caps(text):
        global_cap = min(caps(text))
        items = [line for block in claim_blocks(lines, sections) for line in block.splitlines()]
        for p in sorted(claimed_paths(lines, sections)):
            own = [line for line in items if p in claim_paths(line)]
            if any(re.search(r"wiring|\(new\)", line, re.I) for line in own):
                continue
            local = caps("\n".join(own))
            cap = min(local) if local else global_cap
            try:
                root = pathlib.Path(options.root).resolve()
                if pathlib.PureWindowsPath(p).drive:
                    continue
                target = (root / p.replace("\\", "/")).resolve()
                if not target.is_relative_to(root) or not target.is_file():
                    continue
                with target.open("rb") as stream:
                    count = sum(chunk.count(b"\n") for chunk in iter(
                        lambda: stream.read(65536), b""))
            except (OSError, ValueError, RuntimeError):
                continue
            margin = options.cap_margin_pct
            if count * 100 >= cap * (100 - margin):
                result.append(("B4", "FAIL", f'{p} has {count} lines, within {margin}% of '
                               f'the stated {cap}-line cap: mark its claim "wiring" or split first'))
    return sorted(result, key=lambda r: r[0])
