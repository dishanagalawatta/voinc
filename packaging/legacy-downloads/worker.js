// Kit's old Voinc hosts, all redirected to anoma.ly:
// - downloads.hex.kitlangton.dev and .com (the bucket Voinc 2.3.1 and earlier
//   read): every path, 302, to https://anoma.ly/hex/files, where the site
//   Worker serves the frozen 2.3.1 history from voinc-production-files and the
//   appcast and installers from the site build.
// - voinc.kitlangton.com and .dev (the old website): 301 to https://anoma.ly/hex,
//   with the download and installer paths kept working.
const WEBSITE = {
  "/download": "/voinc/download/macos-arm64-dmg",
  "/install-linux.sh": "/voinc/files/install-linux.sh",
  "/install-windows.ps1": "/voinc/files/install-windows.ps1",
}

export default {
  fetch(request) {
    const { hostname, pathname, search } = new URL(request.url)
    if (hostname.startsWith("downloads.")) {
      return Response.redirect(`https://anoma.ly/hex/files${pathname}${search}`, 302)
    }
    const path = WEBSITE[pathname.replace(/\/+$/, "")] ?? "/voinc/"
    return Response.redirect(`https://anoma.ly${path}`, 301)
  },
}
