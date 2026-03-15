.PHONY: build bench serve clean guest

build:
	cargo build --release

guest: guest/init.c
	musl-gcc -static -Os -march=x86-64 -mno-avx -mno-avx2 -o guest/init guest/init.c

bench: build
	./bench.sh

serve: build
	./target/release/zeroboot serve workdir

clean:
	cargo clean
	rm -rf workdir
