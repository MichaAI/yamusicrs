# Пакет для Arch Linux (.pkg.tar.zst), собранный nix'ом: `nix build .#arch`.
#
# Бинарь — тот же, что и в nix-пакете (профиль dist, LTO), но перепривязанный к системе:
# интерпретатор из FHS вместо /nix/store, rpath вычищен — libpipewire, libgcc_s и glibc
# берутся у Arch, а Vulkan/GL wgpu и так открывает через dlopen по стандартным путям.
# Упаковывает настоящий makepkg из nixpkgs, так что .PKGINFO/.MTREE/.BUILDINFO — штатные.
{
  lib,
  stdenvNoCC,
  yamusicrs,
  pacman,
  fakeroot,
  libarchive,
  zstd,
  patchelf,
  binutils,
  coreutils,
  findutils,
  gnugrep,
  gnused,
  gawk,
  gettext,
  file,
  bash,
}:

let
  inherit (stdenvNoCC.hostPlatform) system;
  archInfo =
    {
      x86_64-linux = {
        carch = "x86_64";
        interpreter = "/lib64/ld-linux-x86-64.so.2";
      };
      aarch64-linux = {
        carch = "aarch64";
        interpreter = "/lib/ld-linux-aarch64.so.1";
      };
    }
    .${system};
in
stdenvNoCC.mkDerivation {
  pname = "yamusicrs-arch";
  inherit (yamusicrs) version;
  dontUnpack = true;

  nativeBuildInputs = [
    pacman
    fakeroot
    libarchive
    zstd
    patchelf
    binutils
    coreutils
    findutils
    gnugrep
    gnused
    gawk
    gettext
    file
    bash
  ];

  buildPhase = ''
    runHook preBuild

    work=$PWD/work
    root=$work/root
    mkdir -p $root/usr/bin
    install -Dm755 ${yamusicrs}/bin/yamusicrs $root/usr/bin/yamusicrs
    cp -r --no-preserve=mode ${yamusicrs}/share $root/usr/share
    chmod 755 $root/usr/bin/yamusicrs

    patchelf --set-interpreter ${archInfo.interpreter} --remove-rpath $root/usr/bin/yamusicrs
    if readelf -d $root/usr/bin/yamusicrs | grep -q /nix/store; then
      echo "в бинаре остались пути /nix/store" >&2; exit 1
    fi
    if grep -rq /nix/store $root/usr/share; then
      echo "в share остались пути /nix/store" >&2; exit 1
    fi

    # glibc из nixpkgs свежий, а libm при каждом релизе версионирует заново (sinh@GLIBC_2.44 и т.п.):
    # минимальную версию для depends берём из самого бинаря, чтобы pacman не дал поставить на старый.
    glibc=$(objdump -T $root/usr/bin/yamusicrs | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1)
    echo "нужен glibc>=$glibc"

    substitute ${./PKGBUILD} $work/PKGBUILD \
      --subst-var-by version ${yamusicrs.version} \
      --subst-var-by carch ${archInfo.carch} \
      --subst-var-by glibc $glibc

    cp --no-preserve=mode ${pacman}/etc/makepkg.conf $work/makepkg.conf
    {
      echo 'CARCH="${archInfo.carch}"'
      echo "PKGEXT='.pkg.tar.zst'"
      echo 'COMPRESSZST=(zstd -c -T0 -19 -)'
      echo "PACKAGER='yamusicrs nix build <nix@yamusicrs.invalid>'"
    } >> $work/makepkg.conf

    # PACMAN=true: makepkg спрашивает у pacman список установленного для .BUILDINFO — в песочнице его нет.
    export HOME=$work PACMAN=true SOURCE_DATE_EPOCH=''${SOURCE_DATE_EPOCH:-1}
    (cd $work && makepkg --config $work/makepkg.conf --nodeps --nocheck --skipinteg --noconfirm -f)

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp work/*.pkg.tar.zst $out/
    runHook postInstall
  '';

  meta = {
    description = "yamusicrs, упакованный для Arch Linux";
    license = lib.licenses.asl20;
    platforms = [
      "x86_64-linux"
      "aarch64-linux"
    ];
  };
}
