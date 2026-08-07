/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  // Class-based, not media-based: the app offers Light / Dark / System, so the
  // decision is made in `lib/theme.ts` and published as one class on <html>.
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        // Dark-mode surfaces. These are the values the app's existing dark
        // chrome (sidebar, settings, output panel) was already hand-written
        // with, named so a new `dark:` variant lands on the same greys instead
        // of inventing a third.
        surface: {
          DEFAULT: '#1e1e1e',
          raised: '#252526',
          hover: '#2a2a2b',
          sunken: '#181818',
        },
        primary: {
          50: '#f0f9ff',
          100: '#e0f2fe',
          200: '#bae6fd',
          300: '#7dd3fc',
          400: '#38bdf8',
          500: '#0ea5e9',
          600: '#0284c7',
          700: '#0369a1',
          800: '#075985',
          900: '#0c4a6e',
        },
      },
      keyframes: {
        shake: {
          '0%, 100%': { transform: 'translateX(0)' },
          '20%, 60%': { transform: 'translateX(-4px)' },
          '40%, 80%': { transform: 'translateX(4px)' },
        },
      },
      animation: {
        shake: 'shake 0.3s ease-in-out',
      },
    },
  },
  plugins: [],
};
