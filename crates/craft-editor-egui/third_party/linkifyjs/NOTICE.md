# Frozen source URL recognition data

`tlds-4.3.3.txt` is a decoded, sorted mirror of the ASCII and internationalized IANA TLD data embedded by linkifyjs4.3.3, used by the frozen Writers Studio editor. It is data, not a JavaScript runtime. The original MIT copyright and permission notice is preserved in LICENSE-MIT.txt.

- Package: linkifyjs4.3.3, upstream https://github.com/nfrasser/linkifyjs/tree/v4.3.3/packages/linkifyjs
- Original source SHA256: `c2235aeb0733ff36f12bda2aa8dc1f5efbfc94315089d5b69bc75934ccea5f9c` for installed dist/linkify.cjs.
- The source identifies https://data.iana.org/TLD/tlds-alpha-by-domain.txt as its data origin. No live TLD update occurs while editing.
- Extraction decodes only the two generated trie strings and sorts/deduplicates complete labels; no author/project content is included.

Explicit-protocol URL recognition is separate from bare-domain/email TLD validation. This mirror does not itself establish scanner compatibility; synthetic actual-source boundary tests are required.
