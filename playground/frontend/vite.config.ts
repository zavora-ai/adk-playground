import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  build: {
    rolldownOptions: {
      output: {
        chunkFileNames: 'assets/[name]-[hash].js',
        codeSplitting: {
          groups: [
            {
              name: 'monaco',
              test: /monaco-editor|@monaco-editor/,
              priority: 40,
            },
            {
              name: 'katex',
              test: /katex/,
              priority: 30,
            },
            {
              name: 'highlight',
              test: /highlight\.js/,
              priority: 30,
            },
            {
              name: 'markdown',
              test: /react-markdown|remark|rehype|unified|mdast|hast|micromark/,
              priority: 20,
            },
            {
              name: 'vendor',
              test: /node_modules/,
              priority: 10,
            },
          ],
        },
      },
    },
  },
  server: {
    port: 3737,
    strictPort: true,
    proxy: {
      '/api': 'http://localhost:9876',
    },
  },
})
