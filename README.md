To use this do

```
$ git clone https://github.com/derekdreery/cargo-docserve
$ pushd cargo-docserve
$ cargo install --path .
$ popd
```

And then from your project somewhere

```
$ cargo docserve
```

If you want to install a newer version do (from the docserve source)

```
$ git pull
$ cargo install --force --path .
```
