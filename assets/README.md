# Logo

| File             | Used for                                                              |
| ---------------- | --------------------------------------------------------------------- |
| `logo.svg`       | App icon, 32 px and larger (`logo.png` is a 1024 px render)            |
| `logo-small.svg` | 16–24 px icons: Windows `.ico` small sizes, tray, browser toolbar      |
| `wordmark.svg`   | Horizontal "supo" logo for light backgrounds                           |

The lettering is set in [Outfit](https://fonts.google.com/specimen/Outfit)
(SIL Open Font License) and converted to outlines, so the SVG files need no
fonts. The "o" doubles as a download button.

To regenerate the app icons after changing the logo:

```sh
cd apps/desktop
npx tauri icon ../../assets/logo.png -o src-tauri/icons
```

Then rebuild `icon.ico` so its 16 and 24 px images come from `logo-small.svg`,
and export `logo-small.svg` at 64 px as `src-tauri/icons/tray.png`.
