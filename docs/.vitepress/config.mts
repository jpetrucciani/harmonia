import { defineConfig } from 'vitepress';

export default defineConfig({
  title: 'harmonia',
  description: 'Poly-repo orchestration with graph-aware workflows',
  cleanUrls: true,
  themeConfig: {
    nav: [
      { text: 'Home', link: '/' },
      { text: 'Getting Started', link: '/getting-started' },
      { text: 'CLI', link: '/cli/' },
      { text: 'Workflows', link: '/workflows' },
      { text: 'Configuration', link: '/configuration' }
    ],
    sidebar: [
      {
        text: 'Introduction',
        items: [
          { text: 'Getting Started', link: '/getting-started' },
          { text: 'Configuration', link: '/configuration' }
        ]
      },
      {
        text: 'Workflow',
        items: [
          { text: 'Core Workflows', link: '/workflows' },
          { text: 'Plan and MR', link: '/plan-and-mr' },
          { text: 'Shell and Completions', link: '/shell' }
        ]
      },
      {
        text: 'Reference',
        items: [
          { text: 'CLI Reference', link: '/cli/' },
          { text: 'Release and Packaging', link: '/release' },
          { text: 'Troubleshooting', link: '/troubleshooting' }
        ]
      }
    ],
    search: {
      provider: 'local'
    },
    footer: {
      message: 'Documentation is evolving with the CLI',
      copyright: 'Harmonia'
    }
  }
});
