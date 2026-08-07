{ pkgs }: {
  deps = [
    pkgs.python312
    pkgs.python312Packages.pip
    # Pillow's build needs these even though the wheel is usually prebuilt;
    # keeping them avoids a source build failing on a missing zlib/jpeg header.
    pkgs.zlib
    pkgs.libjpeg
  ];
}
