{
  codex,
  runCommand,
  python3,
  patch,
}:
codex.overrideAttrs (old: {
  patches = (old.patches or [ ]) ++ [ ./codex-terminal-theme.patch ];
  cargoDeps =
    runCommand "codex-terminal-theme-vendor"
      {
        nativeBuildInputs = [
          python3
          patch
        ];
      }
      ''
        mkdir -p $out
        cp -s ${old.cargoDeps}/Cargo.lock $out/
        cp -rs ${old.cargoDeps}/.cargo $out/
        for source in ${old.cargoDeps}/source-*; do
          if [ -d "$source/crossterm-0.29.0" ]; then
            destination="$out/$(basename "$source")"
            mkdir -p "$destination"
            cp -R "$source/crossterm-0.29.0" "$destination/"
            chmod -R u+w "$destination/crossterm-0.29.0"
            patch -d "$destination/crossterm-0.29.0" -p1 < ${./crossterm-terminal-theme.patch}
            python3 - "$destination/crossterm-0.29.0" <<'PY'
        import hashlib
        import json
        import pathlib
        import sys
        root = pathlib.Path(sys.argv[1])
        path = root / ".cargo-checksum.json"
        checksum = json.loads(path.read_text())
        for name in checksum["files"]:
            checksum["files"][name] = hashlib.sha256((root / name).read_bytes()).hexdigest()
        path.write_text(json.dumps(checksum))
        PY
          else
            ln -s "$source" "$out/"
          fi
        done
      '';
})
