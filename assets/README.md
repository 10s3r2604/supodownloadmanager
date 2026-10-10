# Logo

| File            | Used for                                                        |
| --------------- | --------------------------------------------------------------- |
| `logo.svg`      | Main app icon, 32 px and larger (`logo.png` is a 1024 px render) |
| `logo-small.svg`| 16–24 px icons: Windows `.ico` small sizes, tray, browser toolbar |
| `logo-supo.svg` | Alternative "supo" design                                        |

The lettering is set in [Poppins](https://fonts.google.com/specimen/Poppins)
and [Outfit](https://fonts.google.com/specimen/Outfit) (SIL Open Font License)
and converted to outlines, so the SVG files need no fonts.

To regenerate the app icons after changing the logo:

```sh
cd apps/desktop
npx tauri icon ../../assets/logo.png -o src-tauri/icons
```

Then rebuild `icon.ico` so its 16 and 24 px images come from `logo-small.svg`,
and export `logo-small.svg` at 64 px as `src-tauri/icons/tray.png`.
