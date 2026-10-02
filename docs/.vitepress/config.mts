import { defineConfig } from "vitepress";

// rookey.click/docs: built by site/Dockerfile, served by nginx next to the landing page
export default defineConfig({
  title: "rookey",
  description: "Dictation for Linux, macOS and Windows: hold a key, talk, let go, it's typed.",
  base: "/docs/",
  srcDir: "src",
  cleanUrls: true,
  head: [["link", { rel: "icon", href: "/docs/icon.svg", type: "image/svg+xml" }]],
  // the landing page's dark code blocks, in both modes
  markdown: { theme: "github-dark" },
  themeConfig: {
    // src/public/icon*.svg are copies of src/ui/icon.svg; the header follows the theme toggle, so one fixed-colour file per theme
    logo: { light: "/icon-light.svg", dark: "/icon-dark.svg" },
    siteTitle: "rookey",
    nav: [
      { text: "rookey.click", link: "https://rookey.click" },
      { text: "Download", link: "https://github.com/7KiLL/rookey/releases/latest" },
    ],
    sidebar: [
      {
        text: "Start",
        items: [
          { text: "Install", link: "/install" },
          { text: "Commands", link: "/commands" },
          { text: "Hotkeys", link: "/hotkeys" },
        ],
      },
      {
        text: "Dictating",
        items: [
          { text: "Engines", link: "/engines" },
          { text: "Screen terms", link: "/screen-terms" },
          { text: "While you talk", link: "/while-you-talk" },
          { text: "Settings", link: "/settings" },
        ],
      },
      {
        text: "Around it",
        items: [
          { text: "Status for bars", link: "/status" },
          { text: "Troubleshooting", link: "/troubleshooting" },
          { text: "Compared", link: "/compared" },
        ],
      },
    ],
    outline: { level: [2, 3] },
    search: { provider: "local" },
    editLink: { pattern: "https://github.com/7KiLL/rookey/edit/master/docs/src/:path", text: "Edit this page" },
    socialLinks: [{ icon: "github", link: "https://github.com/7KiLL/rookey" }],
  },
});
