# actr

Swift package distribution repository for ACTR.

- Source of truth: [actr monorepo](https://github.com/Actrium/actr)
- Package sync repository: `Actrium/actr-swift-package-sync`

This repository owns the Swift package release workflows and published package metadata.
The release workflow clones `actr` at the requested source tag with `git clone --depth 1 --branch <tag>`, rebuilds `ActrFFI.xcframework`, updates `Package.swift`, syncs the distributed Swift sources, and publishes the zip asset as a GitHub Release.

Do not copy the monorepo workspace into this repository, and do not develop product code here.
