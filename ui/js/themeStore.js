// js/themeStore.js
// Applies a saved custom (CSS) theme at startup. The upstream community theme
// store (browse/upload via Monochrome accounts) was removed in Volna; custom
// theme CSS is stored locally only.

const GENERIC_FONT_FAMILIES = [
    'serif',
    'sans-serif',
    'monospace',
    'cursive',
    'fantasy',
    'system-ui',
    'inter',
    'ibm plex mono',
    'roboto',
    'open sans',
    'lato',
    'montserrat',
    'poppins',
    'apple music',
    'sf pro display',
    'courier new',
    'times new roman',
    'arial',
    'helvetica',
    'verdana',
    'tahoma',
    'trebuchet ms',
    'impact',
    'gill sans',
];

export class ThemeStore {
    constructor() {
        this.applySavedTheme();
    }

    applySavedTheme() {
        const theme = localStorage.getItem('monochrome-theme');
        const css = localStorage.getItem('custom_theme_css');
        if (theme === 'custom' && css) {
            const metadataStr = localStorage.getItem('community-theme');
            let metadata = null;
            if (metadataStr) {
                try {
                    metadata = JSON.parse(metadataStr);
                } catch (e) {
                    console.warn(e);
                }
            }

            if (metadata) {
                this.applyTheme({
                    css: css,
                    id: metadata.id,
                    name: metadata.name,
                    authorName: metadata.author,
                });
            } else {
                this.applyTheme(css);
            }
        }
    }

    applyTheme(theme) {
        let css = theme.css;
        if (!css && typeof theme === 'string') {
            css = theme;
            theme = { name: 'Custom Theme', authorName: 'Unknown' };
        }

        localStorage.setItem('custom_theme_css', css);
        localStorage.setItem('monochrome-theme', 'custom');

        const metadata = {
            id: theme.id,
            name: theme.name,
            author:
                theme.authorName || theme.expand?.author?.username || theme.expand?.author?.display_name || 'Unknown',
        };
        localStorage.setItem('community-theme', JSON.stringify(metadata));

        let styleEl = document.getElementById('custom-theme-style');
        if (!styleEl) {
            styleEl = document.createElement('style');
            styleEl.id = 'custom-theme-style';
            document.head.appendChild(styleEl);
        }

        const fontMatch = css.match(/--font-family:\s*([^;}]+)/);
        const urlMatch = css.match(/--font-url:\s*([^;}]+)/);

        if (fontMatch && fontMatch[1]) {
            const fontFamilyValue = fontMatch[1].trim();
            const mainFont = fontFamilyValue.split(',')[0].trim().replace(/['"]/g, '');

            const isPresetOrGeneric = GENERIC_FONT_FAMILIES.some((generic) => mainFont.toLowerCase() === generic);

            if (!isPresetOrGeneric) {
                const FONT_LINK_ID = 'monochrome-dynamic-font';
                let link = document.getElementById(FONT_LINK_ID);

                if (urlMatch && urlMatch[1]) {
                    const customUrl = urlMatch[1].trim().replace(/['"]/g, '');
                    console.log(`Applying custom font URL: ${customUrl}`);

                    let isGoogleFontsHost = false;
                    try {
                        const parsedUrl = new URL(customUrl, window.location.href);
                        isGoogleFontsHost = parsedUrl.hostname === 'fonts.googleapis.com';
                    } catch (_e) {
                        isGoogleFontsHost = false;
                    }

                    if (customUrl.match(/\.(css)$/i) || isGoogleFontsHost) {
                        if (!link) {
                            link = document.createElement('link');
                            link.id = FONT_LINK_ID;
                            link.rel = 'stylesheet';
                            document.head.appendChild(link);
                        }
                        link.href = customUrl;
                    } else {
                        if (link) link.remove();
                        const fontFace = `
@font-face {
    font-family: '${mainFont}';
    src: url('${customUrl}');
    font-weight: 100 900;
    font-display: swap;
}
`;
                        css = fontFace + css;
                    }
                } else {
                    console.log(`Applying custom font from theme (Google Fonts): ${mainFont}`);
                    const encodedFamily = encodeURIComponent(mainFont);
                    const url = `https://fonts.googleapis.com/css2?family=${encodedFamily}:wght@100;200;300;400;500;600;700;800;900&display=swap`;

                    if (!link) {
                        link = document.createElement('link');
                        link.id = FONT_LINK_ID;
                        link.rel = 'stylesheet';
                        document.head.appendChild(link);
                    }
                    link.href = url;
                }
            }
        }

        styleEl.textContent = css;

        const root = document.documentElement;
        ['background', 'foreground', 'primary', 'secondary', 'muted', 'border', 'highlight', 'font-family'].forEach(
            (key) => {
                root.style.removeProperty(`--${key}`);
            }
        );
        root.setAttribute('data-theme', 'custom');

        document.querySelectorAll('.theme-option').forEach((el) => el.classList.remove('active'));
        document.querySelector('[data-theme="custom"]')?.classList.add('active');

        // Force reflow to ensure theme changes are applied immediately
        document.documentElement.style.display = 'none';
        // eslint-disable-next-line @typescript-eslint/no-unused-expressions
        document.documentElement.offsetHeight;
        document.documentElement.style.display = '';

        window.dispatchEvent(new CustomEvent('theme-changed', { detail: { theme: 'custom' } }));
    }
}
