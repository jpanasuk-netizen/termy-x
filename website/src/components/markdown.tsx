import { remark } from 'remark';
import remarkGfm from 'remark-gfm';
import remarkRehype from 'remark-rehype';
import { toJsxRuntime } from 'hast-util-to-jsx-runtime';
import {
  Children,
  type ComponentProps,
  type ReactElement,
  type ReactNode,
  Suspense,
  use,
  useDeferredValue,
} from 'react';
import { Fragment, jsx, jsxs } from 'react/jsx-runtime';
import { DynamicCodeBlock } from 'fumadocs-ui/components/dynamic-codeblock.core';
import { createShikiFactory } from 'fumadocs-core/highlight/shiki';
import defaultMdxComponents from 'fumadocs-ui/mdx';
import { visit } from 'unist-util-visit';
import type { ElementContent, Root, RootContent } from 'hast';

interface Processor {
  process: (content: string) => Promise<ReactNode>;
}

function rehypeWrapWords() {
  return (tree: Root) => {
    visit(tree, ['text', 'element'], (node, index, parent) => {
      if (node.type === 'element' && node.tagName === 'pre') return 'skip';
      if (node.type !== 'text' || !parent || index === undefined) return;

      const words = node.value.split(/(?=\s)/);

      const newNodes: ElementContent[] = words.flatMap((word) => {
        if (word.length === 0) return [];

        return {
          type: 'element',
          tagName: 'span',
          properties: {
            class: 'animate-fd-fade-in',
          },
          children: [{ type: 'text', value: word }],
        };
      });

      Object.assign(node, {
        type: 'element',
        tagName: 'span',
        properties: {},
        children: newNodes,
      } satisfies RootContent);
      return 'skip';
    });
  };
}

function createProcessor(): Processor {
  const processor = remark().use(remarkGfm).use(remarkRehype).use(rehypeWrapWords);

  return {
    async process(content) {
      const nodes = processor.parse({ value: content });
      const hast = await processor.run(nodes);

      return toJsxRuntime(hast, {
        development: false,
        jsx,
        jsxs,
        Fragment,
        components: {
          ...defaultMdxComponents,
          pre: Pre,
          img: undefined, // use JSX
        },
      });
    },
  };
}

// The default `fumadocs-ui/components/dynamic-codeblock` export bundles every
// Shiki grammar and theme (~9 MB). Release notes only ever use a handful of
// languages, so build a highlighter limited to those; unknown languages fall
// back to plain text.
const shikiFactory = createShikiFactory({
  async init() {
    const [{ createBundledHighlighter }, { createJavaScriptRegexEngine }] =
      await Promise.all([
        import('shiki/core'),
        import('shiki/engine/javascript'),
      ]);
    const createHighlighter = createBundledHighlighter({
      langs: {
        bash: () => import('@shikijs/langs/bash'),
        sh: () => import('@shikijs/langs/sh'),
        shell: () => import('@shikijs/langs/shell'),
        zsh: () => import('@shikijs/langs/zsh'),
        console: () => import('@shikijs/langs/console'),
        json: () => import('@shikijs/langs/json'),
        toml: () => import('@shikijs/langs/toml'),
        yaml: () => import('@shikijs/langs/yaml'),
        rust: () => import('@shikijs/langs/rust'),
        diff: () => import('@shikijs/langs/diff'),
        ts: () => import('@shikijs/langs/ts'),
        md: () => import('@shikijs/langs/md'),
      },
      themes: {
        'github-light': () => import('@shikijs/themes/github-light'),
        'github-dark': () => import('@shikijs/themes/github-dark'),
      },
      engine: () => createJavaScriptRegexEngine(),
    });
    return createHighlighter({ langs: [], themes: [] });
  },
});

function Pre(props: ComponentProps<'pre'>) {
  const code = Children.only(props.children) as ReactElement;
  const codeProps = code.props as ComponentProps<'code'>;
  const content = codeProps.children;
  if (typeof content !== 'string') return null;

  let lang =
    codeProps.className
      ?.split(' ')
      .find((v) => v.startsWith('language-'))
      ?.slice('language-'.length) ?? 'text';

  if (lang === 'mdx') lang = 'md';

  return (
    <DynamicCodeBlock
      lang={lang}
      code={content.trimEnd()}
      highlighter={() => shikiFactory.getOrInit()}
      options={{ themes: { light: 'github-light', dark: 'github-dark' } }}
    />
  );
}

const processor = createProcessor();

export function Markdown({ text }: { text: string }) {
  const deferredText = useDeferredValue(text);

  return (
    <Suspense fallback={<p className="invisible">{text}</p>}>
      <Renderer text={deferredText} />
    </Suspense>
  );
}

// Module-level cache shared across requests during SSR — keep it bounded so
// the server process doesn't accumulate a React tree per unique release body.
const CACHE_LIMIT = 32;
const cache = new Map<string, Promise<ReactNode>>();

function Renderer({ text }: { text: string }) {
  const result = cache.get(text) ?? processor.process(text);
  cache.delete(text);
  cache.set(text, result);
  if (cache.size > CACHE_LIMIT) {
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }

  return use(result);
}
