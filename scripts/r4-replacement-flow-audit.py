#!/usr/bin/env python3
import argparse
import glob
import json
from collections import Counter, deque


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--original-label-glob", required=True)
    parser.add_argument("--raw-results", required=True)
    args = parser.parse_args()

    cell_need = Counter()
    source_need = Counter()
    for path in sorted(glob.glob(args.original_label_glob)):
        with open(path, encoding="utf-8") as handle:
            for line in handle:
                record = json.loads(line)
                if not record["completed"]:
                    position = record["position"]
                    cell_need[(position["split"], position["phase"])] += 1
                    source_need[position["source"]] += 1

    available = Counter()
    attempts = incomplete = 0
    with open(args.raw_results, encoding="utf-8") as handle:
        for line in handle:
            record = json.loads(line)
            attempts += 1
            if record["completed"]:
                position = record["position"]
                available[
                    (position["split"], position["phase"], position["source"])
                ] += 1
            else:
                incomplete += 1

    cells = sorted(cell_need)
    sources = sorted(source_need)
    source_node = 0
    cell_start = 1
    label_source_start = cell_start + len(cells)
    sink = label_source_start + len(sources)
    capacity = [[0] * (sink + 1) for _ in range(sink + 1)]
    original = {}
    for index, cell in enumerate(cells):
        capacity[source_node][cell_start + index] = cell_need[cell]
        for source_index, label_source in enumerate(sources):
            edge = (cell[0], cell[1], label_source)
            value = available[edge]
            capacity[cell_start + index][label_source_start + source_index] = value
            original[(index, source_index)] = value
    for index, label_source in enumerate(sources):
        capacity[label_source_start + index][sink] = source_need[label_source]

    flow = 0
    while True:
        parent = [None] * (sink + 1)
        queue = deque([source_node])
        while queue:
            node = queue.popleft()
            for next_node, value in enumerate(capacity[node]):
                if next_node != source_node and parent[next_node] is None and value > 0:
                    parent[next_node] = node
                    queue.append(next_node)
        if parent[sink] is None:
            break
        increment = 2**63 - 1
        node = sink
        while node != source_node:
            previous = parent[node]
            increment = min(increment, capacity[previous][node])
            node = previous
        node = sink
        while node != source_node:
            previous = parent[node]
            capacity[previous][node] -= increment
            capacity[node][previous] += increment
            node = previous
        flow += increment

    allocation = {}
    for cell_index, cell in enumerate(cells):
        for source_index, label_source in enumerate(sources):
            used = original[(cell_index, source_index)] - capacity[
                cell_start + cell_index
            ][label_source_start + source_index]
            if used:
                allocation["/".join((*cell, label_source))] = used
    print(
        json.dumps(
            {
                "schema": "haitaka-r4-replacement-flow-audit-v1",
                "attempts": attempts,
                "incomplete": incomplete,
                "incompleteRate": incomplete / attempts,
                "requiredFlow": sum(cell_need.values()),
                "maximumFlow": flow,
                "feasible": flow == sum(cell_need.values()) == sum(source_need.values()),
                "allocation": allocation,
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
