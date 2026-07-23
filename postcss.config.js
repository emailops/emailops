// Tailwind v4 handles vendor prefixing itself (Lightning CSS) — autoprefixer
// is redundant here and was the plugin triggering Vite's "did not pass the
// `from` option to `postcss.parse`" dev warning.
export default {
  plugins: {
    '@tailwindcss/postcss': {},
  },
};
