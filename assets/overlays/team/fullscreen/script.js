let eventSource = new EventSource('/sse/tiltify');

eventSource.onmessage = function (event) {
    try {
        let json = JSON.parse(event.data);
        if (json.event_type === "DonationUpdated") {

        } else {
            if (json.data['total_amount_raised']) {
                let total_amount_raised = json.data['total_amount_raised'].value;
                total_amount_raised = Number.parseFloat(total_amount_raised.replaceAll(/[$,]/g, ''));
                setDonatedAmount(total_amount_raised);
            }
        }

        // {"event_type":"DonationUpdated","datetime":"2025-05-13T01:36:11.351703Z","amount":{"currency":"USD","value":"4.20"},"name":"L00_Cyph3r","message":"Testing in production is fun!"}
    } catch (e) {
        return;
    }

    console.log('Message from server ', event.data);
}

let milestones = [
    50,
    500,
    1000,
    1500,
    2000,
    2500,
    3000,
    3500,
    4000,
    4500,
    5000,
    5500,
    6000,
    6500,
    7000,
    7500,
    8000,
    8500,
    9000,
    9500,
    10000,
    10500,
    11000,
    11500,
    12000,
    12500,
    13000,
    13500,
    14000,
    14500,
    15000,
    15500,
    16000,
    16500,
    17000,
    17500,
    18000,
    18500,
    19000,
    19500,
    20000,
];

function setDonatedAmount(amount, skip_milestones = false) {
    let oldAmount = Number.parseFloat(document.getElementById('total').innerText.replaceAll(/[$,]/g, ''));
    document.getElementById('total').innerText = Intl.NumberFormat('en-US', {
        style: 'currency',
        currency: 'USD',
    }).format(amount);

    if (skip_milestones) {
        return;
    }

    // Check if any milestone has been crossed
    for (let milestone of milestones) {
        if (oldAmount < milestone && amount >= milestone) {
            console.log('Milestone reached:', milestone);
            setTimeout(() => {
                document.getElementById('warbucks').classList.add('shake');
                document.getElementById('warbucks').classList.add('visible');
                (new Audio('/alerts/wb_wow_another_donation.mp3')).play().then();

                setTimeout(() => {
                    document.getElementById('warbucks').classList.remove('shake');
                    document.getElementById('warbucks').classList.remove('visible');
                }, 5000);

            }, 5000);
            break;
        }
    }
}

fetch('/tiltify_team_stats.json').then((res) => {
    res.json().then((json) => {
        let total_amount_raised = json.data.total_amount_raised.value;
        total_amount_raised = Number.parseFloat(total_amount_raised.replaceAll(/[$,]/g, ''));
        setDonatedAmount(total_amount_raised, true);
    })
});