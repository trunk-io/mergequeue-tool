![light-complex (3)](https://github.com/trunk-io/mergequeue/assets/1265982/ded3489b-eef7-482f-b94f-0d944c1d93ce)

### Welcome

This repository hosts the trunk mergequeue-tool which can be used to demonstrate the performance
characteristics of repository managed by trunk merge graph under different simulated loads.

#### How does it work

The load imparted onto the connected queue is controlled by the `mq.toml` file in the .config
folder.

The configuration system allows for setting the desired load on the queue, the flake rate and the
interdependence element of the pull requests.

```toml
# parallelqueue - will push deps information to the service to take advantage of trunk merge dynamic parallel queues
# singlequeue - single traditional queueu
mode = "singlequeue"

# Default value: "none"
#build = "none"

[git]
# Default value: "Jane Doe"
#name = "Jane Doe"

# Default value: "bot@email.com"
#email = "bot@email.com"

[pullrequest]
# Default value: ""
#labels = ""

# Default value: ""
#comment = ""

# Default value: "This pull request was generated by the 'mq' tool"
#body = "This pull request was generated by the 'mq' tool"

# Default value: 10
#requests_per_hour = 10

# Default value: 1
#max_deps = 1

# Default value: 1
#max_impacted_deps = 1

# Default value: 100 (create logical merge conflict every 100 PRs)
#logical_conflict_every = 100

# Default value: "logical-conflict.txt"
#logical_conflict_file = "logical-conflict.txt"

# Default value: ["removed from the merge queue", "To merge this pull request, check the box to the left"]
#detect_stale_pr_comments = ["removed from the merge queue", "To merge this pull request, check the box to the left"]

# Default value: "4 hours"
#close_stale_after = "4 hours"

[test]
# Default value: 0.1
#flake_rate = 0.1

# Default value: "1 second"
#sleep_for = "1 second"

[merge]
# Default value: ""
#labels = ""

# Default value: ""
#comment = ""
```
