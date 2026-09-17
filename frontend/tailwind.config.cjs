/** @type {import('tailwindcss').Config} */
const color = (name) => `var(${name})`;

module.exports = {
  content: [
    "./index.html",
    "./src/**/*.rs",
  ],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        ink: {
          950: color("--bg-primary"),
          900: color("--bg-secondary"),
          800: color("--bg-elevated"),
          700: color("--border-strong"),
          600: color("--color-bg-muted"),
        },
        mint: {
          400: color("--accent"),
          500: color("--accent"),
          600: color("--accent-hover"),
        },
        bull: {
          400: color("--profit"),
          500: color("--profit"),
        },
        bear: {
          400: color("--loss"),
          500: color("--danger"),
        },
        amber: {
          400: color("--warning"),
        },
      },
      fontFamily: {
        sans: ['var(--font-sans)'],
        mono: [
          'var(--font-mono)',
        ],
      },
    },
  },
  plugins: [],
};
