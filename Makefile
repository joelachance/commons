.PHONY: run build test sidecar clean

run: sidecar          ## Launch the panel (Alt+Cmd+Space to summon it)
	cd src-tauri && cargo run

build:
	cd src-tauri && cargo build --release

test:
	cd src-tauri && cargo test

sidecar: sidecar/.venv  ## Local Parakeet environment for dictation
sidecar/.venv:
	cd sidecar && uv venv --python 3.12 && uv pip install -r pyproject.toml

clean:
	cd src-tauri && cargo clean
	rm -rf sidecar/.venv
