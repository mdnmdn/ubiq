## Help is not built for this binary

Ubiq's documentation ships as a bundle beside the binary, and this build has none. Nothing is
broken — the manual is simply not here.

To build it from the content tree in this working copy:

```sh
just help-bundle
```

That validates `help/`, writes `target/help/help.bundle`, and the next time you open this panel
the pages will be in it. `just help-check` validates without writing, and passes trivially when
there is no `help/` folder at all.

A release build carries the bundle already, so seeing this page in one means the bundle did not
make it into the package.
