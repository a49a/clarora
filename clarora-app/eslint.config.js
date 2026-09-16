const js = require("@eslint/js");
const tseslint = require("typescript-eslint");
const reactHooks = require("eslint-plugin-react-hooks");
const prettier = require("eslint-config-prettier");
const globals = require("globals");

module.exports = [
  { ignores: ["**/node_modules/", "windows/", "macos/Pods/", "macos/build/", "android/build/", "android/app/build/", "dist/", "eslint.config.js"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  { languageOptions: { globals: { ...globals.browser } } },
  {
    files: ["**/*.{ts,tsx}"],
    plugins: { "react-hooks": reactHooks },
    rules: {
      // v6 的编译器类规则对旧架构 RN 代码库噪音过大，只保留这两条核心规则。
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
    },
  },
  {
    files: ["**/*.config.js", "**/*.cjs"],
    languageOptions: { globals: { ...globals.node } },
    rules: { "@typescript-eslint/no-require-imports": "off" },
  },
  {
    rules: {
      "@typescript-eslint/no-explicit-any": "off",
      "@typescript-eslint/no-unused-vars": ["warn", { argsIgnorePattern: "^_" }],
      "no-unused-expressions": ["error", { allowShortCircuit: true, allowTernary: true }],
      "no-empty": ["error", { allowEmptyCatch: true }],
    },
  },
  prettier,
];
