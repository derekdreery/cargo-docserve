> **NOTE** This currently uses the `beta` compiler so it can use the 2018 edition. I won't push it
> to crates.io until 2018 stabilizes.

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
