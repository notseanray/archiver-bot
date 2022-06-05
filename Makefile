both: render download

download:
	cd downloader && go build main.go
	-mkdir bin
	mv downloader/main ./bin/downloader

render:
	cd renderer && cargo build --release
	-mkdir bin
	mv renderer/target/release/renderer ./bin
	cp renderer/*.hbs ./bin 
	cp renderer/main.css ./bin

