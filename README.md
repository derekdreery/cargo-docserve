> **NOTE** This currently uses the `beta` compiler so it can use the 2018 edition. I won't push it
> to crates.io until 2018 stabilizes.

To use this do

```
$ cargo +beta install --git https://github.com/derekdreery/cargo-docserve
```

And then from your project somewhere

```
$ cargo docserve
```

If you want to install a newer version do (notice the `--force`):

```
$ cargo +beta install --force --git https://github.com/derekdreery/cargo-docserve
```
