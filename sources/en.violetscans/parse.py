import json

import requests
from bs4 import BeautifulSoup

html = requests.get("https://violetscans.com/comics/", timeout=10).text
soup = BeautifulSoup(html, "html.parser")


def parseDropdowns(title: str) -> dict[str, str]:
    for dropdown in soup.select("div.filter.dropdown"):
        button_text = dropdown.select_one("button.dropdown-toggle").get_text(
            " ", strip=True
        )

        if title.lower() in button_text.lower():
            result = {}

            for li in dropdown.select("ul.dropdown-menu li"):
                inp = li.select_one("input")
                label = li.select_one("label")

                if inp and label:
                    key = inp.get("value")
                    value = label.get_text(strip=True)
                    result[key] = value

            return result

    return {}


genres = parseDropdowns("Genre")
statuses = parseDropdowns("Status")
types = parseDropdowns("Type")
order_by = parseDropdowns("Order by")

print(f"Genres ({len(genres)}): {list(genres.values())}")
print(f"Statuses ({len(statuses)}): {list(statuses.values())}")
print(f"Types ({len(types)}): {list(types.values())}")
print(f"Orderings ({len(order_by)}): {list(order_by.values())}")

genre_export = {"options": list(genres.values()), "ids": list(genres.keys())}
genre_export = json.dumps(genre_export, indent=4)
with open("genres.json", "w") as f:
    f.write(genre_export)
