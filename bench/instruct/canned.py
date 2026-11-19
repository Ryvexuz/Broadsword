"""Hand-written answers that prove the scorer. No model wrote or was called for any of them.

GOOD[case]            an answer that must PASS. The data-derived cases (15, 16, 19) are not here: the tests build their
                      good answer from an independent count over the log in the prompt.
BAD[case]             list of (answer, the check type that must be among the broken rules): partly wrong answers and
                      wrong-format answers (code fence around JSON, intro line, wrong header, ...).
"""
HEADER = [
    "# LOG EXPORT v2.3  |  host=node-07",
    "# Retention: 30 days  --  owner: ops-team",
    "# Format: <timestamp> <LEVEL> <service> <message> id=<n>",
    "# Timezone: UTC (no  daylight saving)",
    "# Contact: logs@example.invalid",
]

GOOD = {
    "01-bullets": "- Catches bugs early\n- Documents expected behaviour\n- Makes refactoring safer\n- Speeds up code review",
    "02-json-keys": '{"title": "The Glass Orchard", "author": "Mira Voss", "year": 1987, "tags": ["mystery", "family", "garden"]}',
    "03-word-limit": "A hash map stores key value pairs and finds values quickly by hashing each key to a bucket index.",
    "04-forbidden-words": "The ocean covers most of the planet and holds enormous life. Its tides are driven by the moon.",
    "05-keep-line": "REF: ZX-9041/b  (do not edit)\nI will definitely write it separate from the rest, and it is occurring tomorrow.",
    "06-table-header": "| Planet | Moons | Rings |\n| --- | --- | --- |\n| Earth | 1 | 0 |\n| Mars | 2 | 0 |\n| Saturn | 146 | 7 |",
    "07-first-last-line": "BEGIN\nSpring brings warm rain and new flowers.\nEND",
    "08-numbered": "1. Fill a pot with water.\n2. Bring it to a boil.\n3. Lower in the egg.\n4. Wait eight minutes.\n5. Cool it in cold water.",
    "09-lowercase-no-commas": "cats sleep for most of the day and hunt at night.",
    "10-exact-answer": "391",
    "11-sorted-lines": "apple\ncherry\nfig\nmango\npear",
    "12-date-format": "2024-02-29",
    "13-char-limit-ascii": "Bean There opens Monday on Main Street with free first coffees for everyone.",
    "14-plain-text": "Recursion is when a function solves a problem by calling itself on a smaller piece of it. It stops when it reaches a simple base case.",
    "17-never-service": "The log records many requests, warnings and a few errors across auth billing search cart mailer and gateway components during one busy morning.",
    "18-keep-header": "\n".join(HEADER) + "\nEND-OF-HEADER",
    "20-lowercase-numbered": "1. gateway shows slow responses and retries\n2. billing hits upstream timeouts",
}

_JSON = '{"title": "The Glass Orchard", "author": "Mira Voss", "year": 1987, "tags": ["mystery", "family", "garden"]}'

BAD = {
    "01-bullets": [
        ("- Catches bugs early\n- Documents expected behaviour\n- Makes refactoring safer", "bullets"),  # 3 bullets
        ("Here are four benefits:\n- Catches bugs early\n- Documents behaviour\n- Safer refactoring\n- Faster review", "bullets"),  # intro line
        ("* Catches bugs early\n* Documents behaviour\n* Safer refactoring\n* Faster review", "bullets"),  # wrong marker
        ("- Catches bugs early because every change is run against the saved expectations again\n- a\n- b\n- c", "bullets"),  # 13 words
    ],
    "02-json-keys": [
        ("```json\n" + _JSON + "\n```", "json_keys"),  # code fence
        ('{"title": "The Glass Orchard", "author": "Mira Voss", "year": "1987", "tags": ["a", "b", "c"]}', "json_keys"),  # year a string
        ('{"title": "The Glass Orchard", "author": "Mira Voss", "year": 1987, "tags": ["a", "b"]}', "json_keys"),  # 2 tags
        ('{"title": "The Glass Orchard", "author": "Mira Voss", "year": 1987, "tags": ["a", "b", "c"], "isbn": "1"}', "json_keys"),  # extra key
        ("Here is the book: " + _JSON, "json_keys"),  # text around
    ],
    "03-word-limit": [
        ("A hash table stores pairs and finds values quickly by hashing each name to a bucket index in the array.", "contains"),
        ("A hash map is a data structure that stores key value pairs and uses a hash function to turn each key into a bucket index so that lookups, inserts and deletes are all fast on average.", "max_words"),
        ("", "min_words"),
    ],
    "04-forbidden-words": [
        ("The ocean is a vast body of water. It covers most of the planet.", "forbidden"),
        ("The ocean covers most of the planet. Its tides follow the moon. It is deep.", "sentences"),
        ("The ocean is Blue and deep. It holds enormous life.", "forbidden"),  # case-insensitive
    ],
    "05-keep-line": [
        ("REF: ZX-9041/b (do not edit)\nI will definitely write it separate from the rest, and it is occurring tomorrow.", "keep_line"),  # one space lost
        ("REF: ZX-9041/b  (do not edit)\nI will definately write it separate from the rest, and it is occurring tomorrow.", "contains"),
        ("Corrected text:\nI will definitely write it separate from the rest, and it is occurring tomorrow.", "keep_line"),  # line dropped
    ],
    "06-table-header": [
        ("| Planet | Moons |\n| --- | --- |\n| Earth | 1 |\n| Mars | 2 |\n| Saturn | 146 |", "table"),
        ("Here is the table:\n| Planet | Moons | Rings |\n| --- | --- | --- |\n| Earth | 1 | 0 |\n| Mars | 2 | 0 |\n| Saturn | 146 | 7 |", "table"),
        ("| Planet | Moons | Rings |\n| --- | --- | --- |\n| Earth | 1 | 0 |\n| Mars | 2 | 0 |", "table"),  # 2 rows
        ("| Planet | Moons | Rings |\n| Earth | 1 | 0 |\n| Mars | 2 | 0 |\n| Saturn | 146 | 7 |", "table"),  # no separator
    ],
    "07-first-last-line": [
        ("Begin\nSpring brings warm rain.\nEND", "first_line"),
        ("BEGIN\nSpring brings warm rain.\nEND\n(done)", "last_line"),
        ("BEGIN\nSpring is here.\nIt is warm.\nEND", "lines_count"),
    ],
    "08-numbered": [
        ("1. Fill a pot.\n2. Boil it.\n3. Add the egg.\n4. Wait.", "numbered"),
        ("Steps:\n1. Fill a pot.\n2. Boil it.\n3. Add the egg.\n4. Wait.\n5. Cool it.", "numbered"),
        ("1) Fill a pot.\n2) Boil it.\n3) Add the egg.\n4) Wait.\n5) Cool it.", "numbered"),
        ("1. Fill a pot.\n2. Boil it.\n4. Add the egg.\n5. Wait.\n6. Cool it.", "numbered"),
    ],
    "09-lowercase-no-commas": [
        ("Cats sleep for most of the day and hunt at night.", "lowercase"),
        ("cats sleep, mostly, and hunt at night.", "forbidden_chars"),
        ("cats sleep a lot. they hunt at night.", "sentences"),
    ],
    "10-exact-answer": [
        ("17 * 23 = 391", "equals"),
        ("392", "equals"),
        ("", "equals"),
    ],
    "11-sorted-lines": [
        ("apple\nfig\ncherry\nmango\npear", "sorted_lines"),
        ("apple\ncherry\nfig\nmango", "lines_count"),
        ("apple, cherry, fig, mango, pear", "lines_count"),
    ],
    "12-date-format": [
        ("2024-02-29.", "full_match"),
        ("29/02/2024", "full_match"),
        ("2024-03-01", "full_match"),
    ],
    "13-char-limit-ascii": [
        ("Bean There opens Monday on Main Street with free first coffees for everyone who comes by this week, so bring a friend too!", "max_chars"),
        ("Bean There is open \U0001F389 come for coffee", "ascii_only"),
        ("Bean There opens Monday #coffee", "forbidden_chars"),
    ],
    "14-plain-text": [
        ("**Recursion** is a function calling itself. It stops at a base case, which ends the loop of calls.", "no_markdown"),
        ("Recursion is a function calling itself on a smaller input. It stops at a base case. That is all of it.", "sentences"),
        ("Recursion is a function calling itself on smaller input. It stops at the base_case.", "no_markdown"),
    ],
    "15-top-warn": [
        ("gateway: 1\nbilling: 1\nsearch: 1", "equals"),
        ("one line only", "lines_count"),
    ],
    "16-json-span": [
        ('```json\n{"first_ts": "a", "last_ts": "b", "total_lines": 1}\n```', "json_keys"),
        ('{"first_ts": "a", "last_ts": "b", "total_lines": 1}', "json_keys"),
    ],
    "17-never-service": [
        ("The services all logged requests and a few warnings during the busy morning in the log file shown here.", "forbidden"),
        ("The log has many requests.", "min_words"),
        ("The log records many requests, warnings and a few errors across auth billing search cart mailer and gateway components during one busy morning, and the system stayed up for all of it with only a few short slowdowns noted, while the on call team watched the dashboards closely and wrote down every odd spike they happened to see.", "max_words"),
    ],
    "18-keep-header": [
        ("\n".join(HEADER[:3] + ["# Timezone: UTC (no daylight saving)", HEADER[4]]) + "\nEND-OF-HEADER", "keep_line"),  # double space collapsed
        ("\n".join(HEADER) + "\nEND", "last_line"),
        ("\n".join(HEADER), "lines_count"),
    ],
    "19-level-table": [
        ("| Level | Count |\n| --- | --- |\n| INFO | 1 |\n| WARN | 2 |\n| ERROR | 3 |", "table"),
        ("| Level | Count |\n| --- | --- |\n| INFO | 1 |", "table"),
    ],
    "20-lowercase-numbered": [
        ("1. Gateway shows slow responses\n2. billing hits timeouts", "lowercase"),
        ("1. gateway shows 3 slow responses\n2. billing hits timeouts", "forbidden_chars"),
        ("gateway shows slow responses and billing hits timeouts", "numbered"),
        ("1. gateway shows slow responses and keeps retrying requests all day long\n2. billing hits upstream timeouts and connection resets while the queue depth stays high for the whole morning and nobody seems to react to it", "max_words"),
    ],
}
