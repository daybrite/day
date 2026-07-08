app-title = Vitrine de Day
counter-value = { $count ->
    [one] { $count } clic
   *[other] { $count } clics
}
decrement = −
increment = +
name-placeholder = Votre nom
greeting = Bonjour, { $name } !
volume-label = Volume
progress-label = Progression
busy-label = Occupé
subscribe-a11y = S'abonner aux mises à jour
flavor-label = Parfum
history-title = Historique
history-entry = le compteur est passé à { $value }
nav-controls = Contrôles
nav-menus = Menus
nav-text = Texte
nav-gauge = Jauge
nav-battery = Batterie
nav-sensors = Capteurs
nav-clipboard = Presse-papiers
nav-network = Réseau
nav-media = Média
nav-shapes = Formes
nav-pickers = Sélecteurs
nav-compose = Composition
nav-files = Fichiers
nav-tabs = Onglets
nav-stack = Pile
nav-list = Liste
nav-webview = Vue Web
nav-lottie = Lottie
nav-about = À propos

shapes-kinds = Types
shapes-transform = Transformation
shapes-angle = Angle
shapes-tap = Toucher pour recolorer
shapes-drag = Glisser pour déplacer

picker-segmented = Segmenté
picker-menu = Menu
picker-inline = Aligné

compose-caption = Pièces de pure composition — sans code natif, sans fonctionnalités cargo, sur tous les backends gratuitement.
compose-rating-label = Note en étoiles
compose-rating-value = Note : { $value } / 5
compose-card-label = Modificateur Carte
compose-card-title = Surface réutilisable
compose-card-body = Marge + arrière-plan + coins arrondis, appliqués comme Modificateur.
compose-badge-label = Pastille superposée
compose-buttons-label = Styles de bouton
compose-plain-btn = Simple
compose-styled-btn = Rempli
compose-env-label = Environnement ambiant
compose-env-value = Teinté par l'accent fourni
list-add = Ajouter 100
list-caption = { $count } lignes — seules les cellules visibles sont créées

webview-url-hint = Saisir une URL
webview-go = Aller
webview-back = Précédent
webview-forward = Suivant
webview-stop = Arrêter
webview-reload = Recharger

lottie-caption = Une animation Lottie native, fournie en JSON (lottie-ios / lottie-android)
lottie-speed = Vitesse
stack-root-body = Une vraie pile push/pop. Son chemin est un signal de l'application.
stack-push = Empiler un détail
stack-detail-title = Niveau { $depth }
stack-detail-body = Empilé sur le chemin. Le bouton retour natif réécrit le dépilement.
tab-one = Aperçu
tab-two = Détails
tab-three = Réglages
tab-one-body = L'onglet aperçu. Chaque onglet conserve son propre état.
tab-two-body = L'onglet détails, sélectionné par sa clé de route.
tab-three-body = L'onglet réglages. Les liens profonds et dayscript choisissent les onglets par clé.
about-text = Une application native multiplateforme construite avec day.
nav-modals = Fenêtres
modal-alert = Afficher l'alerte
modal-confirm = Confirmer
modal-delete = Supprimer…
modal-sheet = Choisir un parfum
modal-prompt = Saisir le nom
alert-title = Avis
alert-body = Vos modifications ont été enregistrées.
ok = OK
confirm-title = Quitter ?
confirm-body = Voulez-vous vraiment quitter ?
delete-title = Supprimer l'élément ?
delete-body = Cette action est irréversible.
delete = Supprimer
flavor-title = Choisissez un parfum
cancel = Annuler
vanilla = vanille
pistachio = pistache

# Files playground (docs/files.md)
files-caption = Sélecteurs de fichiers natifs. « Ouvrir » lit un fichier texte dans l'éditeur ; « Enregistrer » l'écrit.
files-placeholder = Saisissez du texte à enregistrer…
files-open = Ouvrir un fichier…
files-save = Enregistrer le fichier…
files-opened = Ouvert : { $name }

# Battery playground (docs/battery.md)
battery-caption = La part day-part-battery lit la batterie de l'appareil nativement ; le canevas la dessine.
battery-refresh = Lire la batterie
battery-preview = Aperçu
battery-level = Niveau
battery-charging = En charge
battery-reading = Batterie : { $percent } · { $state }
battery-reading-none = Batterie : aucune API batterie sur cette plateforme

# Aire de jeu Capteurs (docs/sensors.md)
sensors-caption = La part day-part-sensors interroge nativement les capteurs de mouvement de l'appareil.
sensors-refresh = Lire les capteurs
sensor-accelerometer = Accéléromètre
sensor-gyroscope = Gyroscope
sensor-magnetometer = Magnétomètre
sensor-reading = x { $x } · y { $y } · z { $z } { $unit }
sensor-waiting = en attente du premier échantillon…
sensor-unavailable = indisponible sur cet appareil

# Aire de jeu Presse-papiers (docs/clipboard.md)
clipboard-caption = La part day-part-clipboard lit et écrit le presse-papiers système nativement.
clipboard-placeholder = Saisissez un texte à copier
clipboard-copy = Copier
clipboard-paste = Coller
clipboard-idle = Presse-papiers intact
clipboard-copied = Copié dans le presse-papiers système
clipboard-copy-failed = Échec de la copie (pas d'API presse-papiers ici)
clipboard-pasted = Collé depuis le presse-papiers système
clipboard-empty = Presse-papiers vide (ou illisible en arrière-plan)

# Aire de jeu Réseau (docs/network.md)
network-caption = La part day-part-network lit nativement l'état de connectivité de l'appareil.
network-refresh = Lire le réseau
network-reading-online = En ligne · { $kind } · facturé : { $expensive }
network-reading-offline = Hors ligne
network-reading-none = Aucune API de connectivité sur cette plateforme

# Aire de jeu Média (docs/media.md)
media-play = Lecture
media-pause = Pause
media-load = Charger

# Aire de jeu Texte (typographie)
text-caption = Les styles sémantiques correspondent aux styles natifs et à l'échelle de texte d'accessibilité.
text-styles-header = Styles
text-weights-header = Graisses
text-styling-header = Gras et italique
text-colors-header = Couleur
text-custom-header = Tailles personnalisées
text-custom-note = Font.System(pt) — mis à l'échelle par la taille de texte d'accessibilité (Dynamic Type).

# Aire de jeu Menus
menus-caption = Menus natifs — la barre de menus de l'application et les menus contextuels par élément — avec sous-menus imbriqués, raccourcis clavier et commandes d'édition standard.
menus-last = Dernière action :
menus-lifecycle = Dernière phase du cycle de vie :
menus-context-hint = Menu contextuel
menus-target = Clic droit ici (appui long sur mobile) pour un menu contextuel
menus-shortcut-hint = Les raccourcis clavier (⌘/Ctrl + touche) apparaissent dans la barre de menus et fonctionnent quand l'application est active — p. ex. Nouveau (N), Enregistrer (S), Recharger (R).

# --- day-part-haptics ---
nav-haptics = Haptique
haptics-caption = Le composant day-part-haptics déclenche un retour haptique natif — chaque bouton joue un motif.
haptics-supported-yes = Moteur haptique disponible sur cette plateforme
haptics-supported-no = Aucun moteur haptique sur cette plateforme (les boutons sont silencieux)
haptics-light = Léger
haptics-medium = Moyen
haptics-heavy = Fort
haptics-success = Succès
haptics-warning = Avertissement
haptics-error = Erreur
haptics-selection = Sélection
haptics-last = Dernier joué
haptics-none = Rien joué pour l'instant
haptics-last-played = Joué : { $style }

# --- day-part-prefs ---
nav-prefs = Préférences
prefs-caption = Conserver une chaîne entre les lancements avec day-part-prefs.
prefs-placeholder = Valeur à mémoriser
prefs-save = Enregistrer
prefs-load = Charger
prefs-clear = Effacer
prefs-idle = Saisissez une valeur, puis Enregistrer.
prefs-empty = (rien d'enregistré)
prefs-saved = Enregistré.
prefs-save-failed = Échec de l'enregistrement.
prefs-loaded = Chargé depuis le stockage.
prefs-missing = Rien d'enregistré pour l'instant.
prefs-cleared = Effacé.
prefs-value-label = Valeur enregistrée :

# --- bundled resources (§18.3) ---
nav-resources = Ressources
resources-caption = Une image chargée par nom depuis une ressource, avec accès aléatoire à des données embarquées.
resources-numbers = numbers.bin : { $len } octets, byte[100] = { $byte }
resources-greeting = greeting.txt : { $text }

# --- day-part-deviceinfo ---
nav-deviceinfo = Appareil
deviceinfo-caption = Identité de l'appareil lue via l'API native de la plateforme (day-part-deviceinfo, sans interface).
deviceinfo-model = Modèle : {$value}
deviceinfo-system = Système : {$name} {$version}
deviceinfo-simulator = Simulateur : {$value}
deviceinfo-yes = oui
deviceinfo-no = non
deviceinfo-refresh = Actualiser

# --- day-piece-activity ---
nav-activity = Activité
activity-caption = Un indicateur indéterminé montre un travail de durée inconnue.
activity-animating = Animation
activity-on = En rotation
activity-off = Arrêté
activity-large-label = Grand

# --- day-piece-searchfield ---
nav-search = Recherche
search-caption = Filtrez la liste en tapant ; l'étiquette de résultat affiche la première correspondance.
search-placeholder = Rechercher un fruit…
search-clear = Effacer

# --- day-piece-map ---
nav-map = Carte
map-caption = Une MKMapView native — plateformes Apple uniquement. Touchez un préréglage pour recentrer la carte en direct.
map-sf = San Francisco
map-nyc = New York
